//! A session over one [`Link`]: pulses and wake gap, liveness, frame dispatch, log decoding with
//! logdata resolution, semihost, and FW update (protocol.md §2–§7).
//!
//! Front ends (TUI, GUI, CLI tools) start a session and consume [`Event`]s.

pub mod fwupdate;
pub mod resolver;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use uclog_link::{CloseReason, Link, LinkEvent};
use uclog_logdata::{Decoders, LogData};
use uclog_proto::frame::{wire, wire_port, PULSE, TYPE_RES};
use uclog_proto::timing::{LIVENESS_TIMEOUT, PULSE_DUE, PULSE_TICK, WAKE_GAP};
use uclog_proto::{cbor, cobs, mux_decode, ports, FrameScanner, Mux, ScanEvent};
use uclog_semihost::{ConsoleOut, Handler, Outcome};

pub use fwupdate::{FwError, FwProgress};
pub use resolver::{ResolverConfig, Source};
pub use uclog_semihost::Config as SemihostConfig;

/// Liveness of the far end (protocol.md §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// Connected, nothing received yet (a hub may be waiting for operator approval).
    Waiting,
    Alive,
    /// No bytes for more than 2 s.
    NotResponding,
}

/// A log line or an error record (protocol.md §7).
#[derive(Debug, Clone, PartialEq)]
pub struct LogRecord {
    /// Seconds since the session started (client receive time, D16).
    pub ts: f64,
    pub target: Option<u8>,
    pub level: String,
    pub file: String,
    pub line: String,
    pub text: String,
    /// Error kind for error records (`framing_invalid`, `no_logdata`, …).
    pub error: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub enum Event {
    Liveness(Liveness),
    Log(LogRecord),
    /// Port-62 device info.
    DeviceInfo(cbor::Value),
    /// Logdata selected for a target, or `None` when nothing was found for the app_hash.
    Logdata {
        app_hash: Vec<u8>,
        source: Option<Source>,
        targets: Vec<u8>,
    },
    /// Port payloads the session does not consume (0, 1, 60, 61, others).
    Port {
        port: u8,
        data: Vec<u8>,
    },
    /// Semihost stdout/stderr.
    Console(ConsoleOut),
    /// The device is waiting for a stdin line (semihost READ on fid 0).
    StdinWanted,
    /// Informational message (resolver notes, etc.).
    Note(String),
    Closed(CloseReason),
}

#[derive(Debug, Clone)]
pub struct Config {
    pub semihost: SemihostConfig,
    pub resolver: ResolverConfig,
    /// Error records for an unknown port are shown once per port, then only counted.
    pub max_frame: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            semihost: SemihostConfig::default(),
            resolver: ResolverConfig::default(),
            max_frame: uclog_proto::MAX_FRAME,
        }
    }
}

enum Cmd {
    Send(u8, Vec<u8>),
    SendRaw(Vec<u8>),
    Stdin(Vec<u8>),
    LoadLogdata(PathBuf, oneshot::Sender<Result<u8, String>>),
    SubscribeFw(oneshot::Sender<mpsc::Receiver<(u8, Vec<u8>)>>),
    AddImage(PathBuf),
    SetFilesDir(Option<PathBuf>),
    Close,
}

/// Cheap, cloneable handle for sending to a running session.
#[derive(Clone)]
pub struct SessionHandle {
    cmd: mpsc::Sender<Cmd>,
}

#[derive(Debug, thiserror::Error)]
#[error("session closed")]
pub struct Closed;

impl SessionHandle {
    /// Send `data` on `port`.
    pub async fn send(&self, port: u8, data: Vec<u8>) -> Result<(), Closed> {
        self.cmd.send(Cmd::Send(port, data)).await.map_err(|_| Closed)
    }

    /// Send a complete payload (frame header included).
    pub async fn send_payload(&self, payload: Vec<u8>) -> Result<(), Closed> {
        self.cmd.send(Cmd::SendRaw(payload)).await.map_err(|_| Closed)
    }

    /// Answer a pending semihost stdin read.
    pub async fn stdin(&self, line: Vec<u8>) -> Result<(), Closed> {
        self.cmd.send(Cmd::Stdin(line)).await.map_err(|_| Closed)
    }

    /// Load a logdata file manually; returns its target digit.
    pub async fn load_logdata(&self, p: PathBuf) -> Result<u8, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Cmd::LoadLogdata(p, tx)).await.map_err(|_| "session closed".to_string())?;
        rx.await.map_err(|_| "session closed".to_string())?
    }

    /// Remember a FW image so its sibling ELF can be found by app_hash.
    pub async fn add_image(&self, p: PathBuf) {
        let _ = self.cmd.send(Cmd::AddImage(p)).await;
    }

    /// Run a FW update.
    pub async fn fw_update(&self, image: &[u8], progress: impl Fn(FwProgress)) -> Result<u8, FwError> {
        fwupdate::run(self, image, progress).await
    }

    /// Change the semihost sandbox directory (open files are closed).
    pub async fn set_files_dir(&self, dir: Option<PathBuf>) {
        let _ = self.cmd.send(Cmd::SetFilesDir(dir)).await;
    }

    pub async fn close(&self) {
        let _ = self.cmd.send(Cmd::Close).await;
    }

    async fn subscribe_fw(&self) -> Result<mpsc::Receiver<(u8, Vec<u8>)>, Closed> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Cmd::SubscribeFw(tx)).await.map_err(|_| Closed)?;
        rx.await.map_err(|_| Closed)
    }
}

/// Start a session on an open link. Events arrive on the returned receiver until `Closed`.
pub fn start(link: Link, cfg: Config) -> (SessionHandle, mpsc::Receiver<Event>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(256);
    let (ev_tx, ev_rx) = mpsc::channel(4096);
    let (out_tx, out_rx) = mpsc::channel(256);
    tokio::spawn(writer(link.tx.clone(), out_rx));
    tokio::spawn(Session::new(cfg, ev_tx, out_tx).run(link, cmd_rx));
    (SessionHandle { cmd: cmd_tx }, ev_rx)
}

/// Writes frames to the link: a pulse after 0.35 s of TX idle (so no gap exceeds 0.5 s), and after
/// more than 2 s of TX idle a wake pulse followed by at least 50 ms before the next frame
/// (protocol.md §2.3).
async fn writer(link: mpsc::Sender<Vec<u8>>, mut out: mpsc::Receiver<Vec<u8>>) {
    let mut last_tx: Option<Instant> = None;
    // Frames may not be sent before this instant (end of the wake gap).
    let mut ready = Instant::now();
    let mut tick = tokio::time::interval(PULSE_TICK);
    let idle = |t: Option<Instant>| t.is_none_or(|t| t.elapsed() > LIVENESS_TIMEOUT);
    loop {
        tokio::select! {
            f = out.recv() => {
                let Some(f) = f else { return };
                if idle(last_tx) {
                    if link.send(PULSE.to_vec()).await.is_err() { return; }
                    ready = Instant::now() + WAKE_GAP;
                }
                tokio::time::sleep_until(ready.into()).await;
                tracing::debug!(target: "uclog::wire", "tx {:>4} {}", f.len(), hex(&f));
                if link.send(f).await.is_err() { return; }
                last_tx = Some(Instant::now());
            }
            _ = tick.tick() => {
                if last_tx.is_none_or(|t| t.elapsed() >= PULSE_DUE) {
                    if idle(last_tx) {
                        ready = Instant::now() + WAKE_GAP;
                    }
                    if link.send(PULSE.to_vec()).await.is_err() { return; }
                    last_tx = Some(Instant::now());
                }
            }
        }
    }
}

struct Session {
    cfg: Config,
    ev: mpsc::Sender<Event>,
    out: mpsc::Sender<Vec<u8>>,
    start: Instant,
    scanner: FrameScanner,
    decoders: Decoders,
    semihost: Handler,
    stdin_wanted: Option<usize>,
    fw: Option<mpsc::Sender<(u8, Vec<u8>)>>,
    liveness: Liveness,
    last_rx: Option<Instant>,
    unknown_ports: HashMap<u8, u64>,
    /// Log frames held while logdata for a new app_hash is being resolved.
    pending: Option<VecDeque<(u8, u32, Vec<u8>)>>,
    resolve_tx: mpsc::Sender<(Vec<u8>, resolver::Outcome)>,
    resolve_rx: Option<mpsc::Receiver<(Vec<u8>, resolver::Outcome)>>,
    app_hash: Option<Vec<u8>>,
    /// Log frames are held until this instant while the device info we asked for is awaited.
    info_wait: Option<Instant>,
}

const PENDING_MAX: usize = 5000;
/// How long log frames are held after asking for the device info (F7).
const INFO_WAIT: Duration = Duration::from_secs(1);

impl Session {
    fn new(cfg: Config, ev: mpsc::Sender<Event>, out: mpsc::Sender<Vec<u8>>) -> Self {
        let (resolve_tx, resolve_rx) = mpsc::channel(8);
        let semihost = Handler::new(cfg.semihost.clone());
        Session {
            scanner: FrameScanner::new(cfg.max_frame),
            cfg,
            ev,
            out,
            start: Instant::now(),
            decoders: Decoders::default(),
            semihost,
            stdin_wanted: None,
            fw: None,
            liveness: Liveness::Waiting,
            last_rx: None,
            unknown_ports: HashMap::new(),
            pending: None,
            resolve_tx,
            resolve_rx: Some(resolve_rx),
            app_hash: None,
            info_wait: None,
        }
    }

    async fn emit(&self, e: Event) {
        let _ = self.ev.send(e).await;
    }

    fn record(
        &self,
        target: Option<u8>,
        level: &str,
        file: &str,
        line: &str,
        text: String,
        error: Option<&'static str>,
    ) -> LogRecord {
        LogRecord {
            ts: self.start.elapsed().as_secs_f64(),
            target,
            level: level.into(),
            file: file.into(),
            line: line.into(),
            text,
            error,
        }
    }

    async fn error(&self, kind: &'static str, text: String, target: Option<u8>) {
        self.emit(Event::Log(self.record(target, "ERROR", "<uclog>", "", text, Some(kind)))).await;
    }

    async fn run(mut self, mut link: Link, mut cmd: mpsc::Receiver<Cmd>) {
        // Manual logdata applies before any device info arrives.
        for p in self.cfg.resolver.manual.clone() {
            match LogData::load(&p) {
                Ok(ld) => self.decoders.insert(Arc::new(ld)),
                Err(e) => self.emit(Event::Note(format!("{}: {e}", p.display()))).await,
            }
        }
        self.emit(Event::Liveness(Liveness::Waiting)).await;
        // Direct links ask for the device info (F7, protocol.md §5.2); a hub asks when it opens
        // the device and replays it to each client.
        if !link.spec.is_hub() {
            let _ = self.out.send(wire_port(ports::DEVICE_INFO, &[])).await;
            // Log frames already on their way would arrive before the reply; hold them so they
            // decode with the logdata it selects.
            if self.decoders.is_empty() {
                self.pending = Some(VecDeque::new());
                self.info_wait = Some(Instant::now() + INFO_WAIT);
            }
        }
        let mut resolve_rx = self.resolve_rx.take().unwrap();
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                ev = link.rx.recv() => match ev {
                    Some(LinkEvent::Data(d)) => self.on_data(&d).await,
                    Some(LinkEvent::Closed(r)) => { self.emit(Event::Closed(r)).await; return; }
                    None => { self.emit(Event::Closed(CloseReason::Eof)).await; return; }
                },
                c = cmd.recv() => match c {
                    None | Some(Cmd::Close) => { self.emit(Event::Closed(CloseReason::Eof)).await; return; }
                    Some(c) => self.on_cmd(c).await,
                },
                r = resolve_rx.recv() => if let Some((hash, outcome)) = r { self.on_resolved(hash, outcome).await },
                _ = tick.tick() => {
                    if self.info_wait.is_some_and(|t| Instant::now() >= t) {
                        // No device info (firmware without F7): decode what was held.
                        self.info_wait = None;
                        if self.app_hash.is_none() {
                            for (t, a, args) in self.pending.take().unwrap_or_default() {
                                self.decode_log(t, a, &args).await;
                            }
                        }
                    }
                    if self.liveness == Liveness::Alive && self.last_rx.is_some_and(|t| t.elapsed() > LIVENESS_TIMEOUT) {
                        self.liveness = Liveness::NotResponding;
                        self.emit(Event::Liveness(Liveness::NotResponding)).await;
                    }
                }
            }
        }
    }

    async fn send_payload(&self, payload: &[u8]) {
        let _ = self.out.send(wire(payload)).await;
    }

    async fn on_cmd(&mut self, c: Cmd) {
        match c {
            Cmd::Send(port, data) if data.len() > uclog_proto::MAX_PORT_DATA => {
                let text = format!(
                    "send: {} bytes for port {port} exceed the {}-byte limit (protocol.md §2.4); not sent",
                    data.len(),
                    uclog_proto::MAX_PORT_DATA
                );
                self.error("send_oversize", text, None).await;
            }
            Cmd::Send(port, data) => {
                let _ = self.out.send(wire_port(port, &data)).await;
            }
            Cmd::SendRaw(p) if p.len() > uclog_proto::MAX_PAYLOAD => {
                let text = format!(
                    "send: {}-byte payload exceeds the {}-byte limit (protocol.md §2.4); not sent",
                    p.len(),
                    uclog_proto::MAX_PAYLOAD
                );
                self.error("send_oversize", text, None).await;
            }
            Cmd::SendRaw(p) => self.send_payload(&p).await,
            Cmd::Stdin(line) => match self.stdin_wanted.take() {
                Some(max) => {
                    let r = self.semihost.stdin(&line, max);
                    let _ = self.out.send(wire_port(ports::SEMIHOST, &r)).await;
                }
                None => self.emit(Event::Note("no semihost stdin read is pending".into())).await,
            },
            Cmd::LoadLogdata(p, reply) => {
                let r = match LogData::load(&p) {
                    Ok(ld) => {
                        let t = ld.target();
                        self.decoders.insert(Arc::new(ld));
                        if !self.cfg.resolver.manual.contains(&p) {
                            self.cfg.resolver.manual.push(p);
                        }
                        Ok(t)
                    }
                    Err(e) => Err(format!("{}: {e}", p.display())),
                };
                let _ = reply.send(r);
            }
            Cmd::SubscribeFw(reply) => {
                let (tx, rx) = mpsc::channel(64);
                self.fw = Some(tx);
                let _ = reply.send(rx);
            }
            Cmd::AddImage(p) => {
                if !self.cfg.resolver.images.contains(&p) {
                    self.cfg.resolver.images.push(p);
                }
            }
            Cmd::SetFilesDir(d) => self.semihost.set_files_dir(d),
            Cmd::Close => {}
        }
    }

    async fn on_data(&mut self, d: &[u8]) {
        tracing::debug!(target: "uclog::wire", "rx {:>4} {}", d.len(), hex(d));
        self.last_rx = Some(Instant::now());
        if self.liveness != Liveness::Alive {
            self.liveness = Liveness::Alive;
            self.emit(Event::Liveness(Liveness::Alive)).await;
        }
        let mut frames = vec![];
        self.scanner.feed_with(d, |e| frames.push(e));
        for e in frames {
            match e {
                ScanEvent::Frame(f) => self.on_frame(&f).await,
                ScanEvent::Oversize(n) => {
                    self.error(
                        "framing_oversize",
                        format!("framing: frame exceeded {} bytes, discarded {n} bytes", self.cfg.max_frame),
                        None,
                    )
                    .await
                }
            }
        }
    }

    async fn on_frame(&mut self, enc: &[u8]) {
        let payload = match cobs::decode(enc) {
            Ok(p) => p,
            Err(_) => {
                let near: String = enc.iter().take(16).map(|b| format!("{b:02x}")).collect();
                return self
                    .error("framing_invalid", format!("framing: invalid COBS ({} bytes) near {near}…", enc.len()), None)
                    .await;
            }
        };
        match mux_decode(&payload) {
            Mux::Pulse => {}
            Mux::ShortLog(p) => {
                self.error("short_log", format!("decode: log frame too short ({} bytes)", p.len()), None).await
            }
            Mux::Log { ty, target, addr, args } => {
                if ty == TYPE_RES {
                    return self
                        .error(
                            "reserved_type",
                            format!("decode: reserved frame type 2 [{}]", hex(&payload)),
                            Some(target),
                        )
                        .await;
                }
                if let Some(p) = &mut self.pending {
                    if p.len() < PENDING_MAX {
                        p.push_back((target, addr, args.to_vec()));
                        return;
                    }
                }
                self.decode_log(target, addr, args).await;
            }
            Mux::Port { port, data } => self.on_port(port, data.to_vec()).await,
        }
    }

    async fn decode_log(&self, target: u8, addr: u32, args: &[u8]) {
        let Some(ld) = self.decoders.get(target) else {
            return self
                .error(
                    "no_logdata",
                    format!("decode: no logdata for target {target} (addr 0x{addr:08x}) [{}]", hex(args)),
                    Some(target),
                )
                .await;
        };
        let rec = match ld.decode(addr, args) {
            Ok(d) => self.record(Some(target), &d.level, &d.file, &d.line, d.text, None),
            Err(e) => {
                let text = match &e {
                    uclog_logdata::DecodeError::UnknownFormat { addr } => {
                        format!("decode: unknown format addr 0x{addr:08x} in {} [{}]", ld.source, hex(args))
                    }
                    e => format!("decode: {e}"),
                };
                self.record(Some(target), "ERROR", "<uclog>", "", text, Some(e.kind()))
            }
        };
        self.emit(Event::Log(rec)).await;
    }

    async fn on_port(&mut self, port: u8, data: Vec<u8>) {
        match port {
            ports::SEMIHOST => self.on_semihost(&data).await,
            ports::DEVICE_INFO => {
                // A device that just booted announces itself: a stdin READ from before is stale.
                self.stdin_wanted = None;
                self.on_device_info(&data).await
            }
            ports::FW | ports::FW_LEGACY if self.fw.is_some() => {
                if self.fw.as_ref().unwrap().send((port, data)).await.is_err() {
                    self.fw = None;
                }
            }
            ports::COMMANDS | ports::PRISM | ports::FW | ports::FW_LEGACY | ports::PLOT | ports::TERMINAL => {
                self.emit(Event::Port { port, data }).await
            }
            _ => {
                let n = self.unknown_ports.entry(port).or_insert(0);
                *n += 1;
                if *n == 1 {
                    self.error("unknown_port", format!("port {port}: no handler ({} bytes)", data.len()), None).await;
                }
                self.emit(Event::Port { port, data }).await;
            }
        }
    }

    async fn on_semihost(&mut self, req: &[u8]) {
        // The device sends one request at a time: a new one means the stdin READ is over (the
        // device was reset).
        self.stdin_wanted = None;
        match self.semihost.handle(req) {
            Outcome::Done { reply, console } => {
                if let Some(c) = console {
                    self.emit(Event::Console(c)).await;
                }
                if let Some(r) = reply {
                    let _ = self.out.send(wire_port(ports::SEMIHOST, &r)).await;
                }
            }
            Outcome::WaitStdin { max } => {
                self.stdin_wanted = Some(max);
                self.emit(Event::StdinWanted).await;
            }
        }
    }

    async fn on_device_info(&mut self, data: &[u8]) {
        let info = match cbor::from_slice(data) {
            Ok(v) => v,
            Err(e) => {
                return self.error("bad_cbor", format!("port 62: invalid CBOR ({e}) [{}]", hex(data)), None).await
            }
        };
        self.emit(Event::DeviceInfo(info.clone())).await;
        let Some(hash) = cbor::get_bytes(&info, "app_hash").map(<[u8]>::to_vec) else {
            return self.emit(Event::Note("device info has no app_hash".into())).await;
        };
        if self.app_hash.as_ref() == Some(&hash) && !self.decoders.is_empty() {
            return;
        }
        self.app_hash = Some(hash.clone());
        self.info_wait = None;
        // Hold log frames until resolution finishes so they decode with the right logdata (and
        // keep those held while waiting for this device info).
        self.pending.get_or_insert_with(VecDeque::new);
        let cfg = self.cfg.resolver.clone();
        let tx = self.resolve_tx.clone();
        tokio::spawn(async move {
            let o = resolver::resolve(&cfg, &hash).await;
            let _ = tx.send((hash, o)).await;
        });
    }

    async fn on_resolved(&mut self, hash: Vec<u8>, o: resolver::Outcome) {
        if self.app_hash.as_ref() != Some(&hash) {
            return; // superseded by a newer device info
        }
        for n in o.notes {
            self.emit(Event::Note(n)).await;
        }
        let (source, targets) = match o.resolved {
            Some(r) => {
                self.decoders.clear();
                let t = r.logdata.iter().map(|l| l.target()).collect();
                for ld in r.logdata {
                    self.decoders.insert(ld);
                }
                (Some(r.source), t)
            }
            None => {
                // The old image's logdata does not describe this one: its logs are `no_logdata`
                // errors (protocol.md §6.4).
                self.decoders.clear();
                (None, vec![])
            }
        };
        self.emit(Event::Logdata { app_hash: hash, source, targets }).await;
        if let Some(p) = self.pending.take() {
            for (t, a, args) in p {
                self.decode_log(t, a, &args).await;
            }
        }
    }
}

fn hex(b: &[u8]) -> String {
    let mut s: String = b.iter().take(32).map(|x| format!("{x:02x}")).collect();
    if b.len() > 32 {
        s.push('…');
    }
    s
}
