//! `loopback:` — a simulated device, for testing hubs and clients without hardware (hub plan §10).
//!
//! Options: `loopback:` or `loopback:rate=5,board=sim` (comma-separated `key=value`):
//! - `rate`: logs per second (default 2; 0 = only replies and pulses)
//! - `board`: board name in the device info (default `loopback`)
//!
//! It logs with the formats of `spec/vectors/sample.logdata` (target digit 1), so decode with
//! `-l spec/vectors/sample.logdata`. Its device info carries [`APP_HASH`].
//!
//! Commands, as text lines ending in `\r` on port 61 (terminal) or port 0; replies go to the same
//! port:
//! - `help`, `echo <text>`
//! - `reset`: new device info and a "log-init"-style log, as after a device reset
//! - `burst <n>`: n logs at once
//! - `big`: one port frame with the maximum 1443 bytes of data
//! - `quiet <secs>`: send nothing at all (no pulses either) for that long
//! - `sh`: semihost requests on port 63 (TIMEOFDAY, then a WRITE to stdout with a reply); the
//!   results are reported as text
//! - `read`: a semihost stdin READ; the reply is reported as text
//! - `plot` / `plot stop`: create plots on port 60 (protocol.md §5.6) and stream data to them, 20
//!   times a second: a line plot `loopback` with two y-axes (`sin`, `cos`; `ramp` on y2), a
//!   spectrogram `spectrum` (32 rows) and an 8×8 heatmap `grid`
//!
//! A CBOR map on port 0 is a request (`{"cmd": …}`, as the hosts' command buttons and Python's
//! `command()` send), answered with a map whose `rsp` is the `cmd`: `get-info` (`board`, `logs`:
//! logs sent so far), `echo` (`value`: the request's `value`), else `error`.

use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{interval, sleep_until, Instant};
use uclog_proto::frame::{mux_decode, wire, wire_port, Mux};
use uclog_proto::scanner::{FrameScanner, ScanEvent};
use uclog_proto::{cbor, ports, MAX_PORT_DATA};

use crate::{channels, Link, LinkError, LinkEvent, LinkSpec};

/// `app_hash` in the loopback device info.
pub const APP_HASH: [u8; 64] = [0x5A; 64];

/// Log format addresses in `spec/vectors/sample.logdata`.
const FMT_HELLO: u32 = 0xF010_0000; // "Hello World! %s"
const FMT_TEMP: u32 = 0xF010_0040; // "temp=%d mV=%u flags=0x%08x"
const FMT_NO_ARGS: u32 = 0xF010_0280; // "no args"

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub rate: f64,
    pub board: String,
}

impl Default for Options {
    fn default() -> Self {
        Options { rate: 2.0, board: "loopback".into() }
    }
}

impl Options {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut o = Options::default();
        for kv in s.split(',').filter(|kv| !kv.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or_else(|| format!("expected key=value, got {kv:?}"))?;
            match k {
                "rate" => o.rate = v.parse().map_err(|_| format!("bad rate {v:?}"))?,
                "board" => o.board = v.into(),
                _ => return Err(format!("unknown option {k:?} (rate, board)")),
            }
        }
        Ok(o)
    }
}

pub(crate) fn open(spec: LinkSpec, opts: &str) -> Result<Link, LinkError> {
    let o = Options::parse(opts).map_err(LinkError::NotFound)?;
    let (tx, txr, ev, evr) = channels();
    tokio::spawn(Device::new(o.clone(), ev).run(txr));
    Ok(Link { description: format!("loopback ({})", o.board), spec, tx, rx: evr })
}

fn log(addr: u32, args: &[u8]) -> Vec<u8> {
    let mut p = addr.to_le_bytes().to_vec();
    p.extend_from_slice(args);
    wire(&p)
}

fn device_info(board: &str) -> Vec<u8> {
    use cbor::Value;
    let m = cbor::map([
        ("app_hash", Value::Bytes(APP_HASH.to_vec())),
        ("board", Value::Text(board.into())),
        ("image_type", Value::Text("app".into())),
    ]);
    wire_port(ports::DEVICE_INFO, &cbor::to_vec(&m))
}

/// A pending semihost request.
enum Pending {
    TimeOfDay { port: u8 },
    Write { port: u8 },
    Read { port: u8 },
}

struct Device {
    o: Options,
    ev: mpsc::Sender<LinkEvent>,
    seq: u32,
    quiet_until: Option<Instant>,
    last_tx: Instant,
    pending: Option<(Pending, Instant)>,
    line: Vec<u8>,
    /// `plot` running: samples sent so far.
    plot: Option<u64>,
}

fn f64s(v: impl IntoIterator<Item = f64>) -> cbor::Value {
    cbor::Value::Array(v.into_iter().map(cbor::Value::Float).collect())
}

fn texts(v: &[&str]) -> cbor::Value {
    cbor::Value::Array(v.iter().map(|s| cbor::Value::Text((*s).into())).collect())
}

/// Port-60 messages that create the demo plots.
fn plot_create() -> Vec<cbor::Value> {
    use cbor::Value::{Integer, Text};
    vec![
        cbor::map([
            ("plot", Text("loopback".into())),
            ("xlabel", Text("t (s)".into())),
            ("ylabel", texts(&["value", "ramp (%)"])),
            ("yser", texts(&["sin", "cos"])),
            ("y2ser", texts(&["ramp"])),
            ("y2limits", f64s([0.0, 100.0])),
            ("xscroll", Integer(400.into())),
        ]),
        cbor::map([
            ("spectrogram", Text("spectrum".into())),
            ("heatmap_rows", Integer(32.into())),
            ("heatmap_cols", Integer(100.into())),
            ("xlabel", Text("t (s)".into())),
            ("ylabel", texts(&["bin"])),
            ("yser", texts(&["power"])),
        ]),
        cbor::map([
            ("heatmap", Text("grid".into())),
            ("heatmap_rows", Integer(8.into())),
            ("heatmap_cols", Integer(8.into())),
            ("xlabel", Text("column".into())),
            ("ylabel", texts(&["row"])),
            ("yser", texts(&["level"])),
        ]),
    ]
}

/// Port-60 data for the 50 ms step `n`: 5 line-plot samples, a spectrogram column, and every
/// 20th step a new heatmap.
fn plot_data(n: u64) -> Vec<cbor::Value> {
    use cbor::Value::{Float, Text};
    let t: Vec<f64> = (0..5).map(|i| (n * 5 + i) as f64 * 0.01).collect();
    let mut out = vec![cbor::map([
        ("plot", Text("loopback".into())),
        ("x", f64s(t.iter().copied())),
        ("y11", f64s(t.iter().map(|t| (t * 6.0).sin()))),
        ("y12", f64s(t.iter().map(|t| (t * 6.0).cos() * 0.5))),
        ("y21", f64s(t.iter().map(|t| (t * 20.0) % 100.0))),
    ])];
    let ts = n as f64 * 0.05;
    let peak = 16.0 + 12.0 * (ts * 0.8).sin();
    out.push(cbor::map([
        ("spectrogram", Text("spectrum".into())),
        ("x", Float(ts)),
        ("y11", f64s((0..32).map(|r| (-((r as f64 - peak) / 3.0).powi(2)).exp() * 100.0))),
    ]));
    if n.is_multiple_of(20) {
        let k = (n / 20) as f64;
        out.push(cbor::map([
            ("heatmap", Text("grid".into())),
            ("x", Float(ts)),
            ("y11", f64s((0..64).map(|i| (((i / 8) as f64 + k) * 0.7).sin() + ((i % 8) as f64 * 0.9).cos()))),
        ]));
    }
    out
}

impl Device {
    fn new(o: Options, ev: mpsc::Sender<LinkEvent>) -> Self {
        Device { o, ev, seq: 0, quiet_until: None, last_tx: Instant::now(), pending: None, line: vec![], plot: None }
    }

    async fn send(&mut self, b: Vec<u8>) -> bool {
        if self.quiet_until.is_some_and(|q| Instant::now() < q) {
            return true;
        }
        self.last_tx = Instant::now();
        self.ev.send(LinkEvent::Data(b)).await.is_ok()
    }

    async fn text(&mut self, port: u8, s: &str) -> bool {
        self.send(wire_port(port, s.as_bytes())).await
    }

    async fn one_log(&mut self) -> bool {
        self.seq = self.seq.wrapping_add(1);
        let b = match self.seq % 3 {
            0 => log(FMT_HELLO, format!("loopback #{}\0", self.seq).as_bytes()),
            1 => {
                let mut a = (self.seq as i32 * 7 - 20).to_le_bytes().to_vec();
                a.extend_from_slice(&(3300 + self.seq % 100).to_le_bytes());
                a.extend_from_slice(&self.seq.to_le_bytes());
                log(FMT_TEMP, &a)
            }
            _ => log(FMT_NO_ARGS, &[]),
        };
        self.send(b).await
    }

    async fn run(mut self, mut rx: mpsc::Receiver<Vec<u8>>) {
        let info = device_info(&self.o.board);
        if !self.send(info).await || !self.send(log(FMT_HELLO, b"loopback start\0")).await {
            return;
        }
        let mut scanner = FrameScanner::default();
        let mut tick = interval(Duration::from_millis(50));
        let period = (self.o.rate > 0.0).then(|| Duration::from_secs_f64(1.0 / self.o.rate));
        let mut next_log = Instant::now() + period.unwrap_or(Duration::from_secs(3600));
        loop {
            tokio::select! {
                d = rx.recv() => {
                    let Some(d) = d else { return };
                    let mut frames = vec![];
                    scanner.feed_with(&d, |e| if let ScanEvent::Frame(f) = e { frames.push(f) });
                    for f in frames {
                        if !self.on_frame(&f).await { return; }
                    }
                }
                _ = sleep_until(next_log), if period.is_some() => {
                    next_log += period.unwrap();
                    if !self.one_log().await { return; }
                }
                _ = tick.tick() => {
                    if let Some(n) = self.plot {
                        self.plot = Some(n + 1);
                        for m in plot_data(n) {
                            if !self.send(wire_port(ports::PLOT, &cbor::to_vec(&m))).await { return; }
                        }
                    }
                    if self.quiet_until.is_some_and(|q| Instant::now() >= q) {
                        self.quiet_until = None;
                    }
                    if let Some((p, t)) = &self.pending {
                        if t.elapsed() > Duration::from_secs(5) {
                            let port = match p { Pending::TimeOfDay { port } | Pending::Write { port } | Pending::Read { port } => *port };
                            if matches!(p, Pending::Read { .. }) && t.elapsed() < Duration::from_secs(60) {
                                // stdin waits for a person
                            } else {
                                self.pending = None;
                                if !self.text(port, "sh: no reply within the timeout\r\n").await { return; }
                            }
                        }
                    }
                    if self.last_tx.elapsed() >= Duration::from_millis(500) && !self.send(vec![0]).await {
                        return;
                    }
                    if self.ev.is_closed() { return; }
                }
            }
        }
    }

    async fn on_frame(&mut self, enc: &[u8]) -> bool {
        let Ok(p) = uclog_proto::cobs::decode(enc) else { return true };
        match mux_decode(&p) {
            Mux::Port { port: ports::SEMIHOST, data } => self.on_semihost(data).await,
            // F7: an empty port-62 frame asks for the device info.
            Mux::Port { port: ports::DEVICE_INFO, data: [] } => {
                let info = device_info(&self.o.board);
                self.send(info).await
            }
            Mux::Port { port: 0, data } if matches!(data.first(), Some(0xa0..=0xbf)) => self.cbor_command(data).await,
            Mux::Port { port: port @ (0 | ports::TERMINAL), data } => {
                let mut ok = true;
                for &b in data {
                    if b == b'\r' || b == b'\n' {
                        let line = String::from_utf8_lossy(&std::mem::take(&mut self.line)).trim().to_string();
                        if !line.is_empty() {
                            ok &= self.command(port, &line).await;
                        }
                    } else {
                        self.line.push(b);
                    }
                }
                ok
            }
            _ => true,
        }
    }

    async fn cbor_command(&mut self, data: &[u8]) -> bool {
        use cbor::Value;
        let req = cbor::from_slice(data).ok();
        let cmd = req.as_ref().and_then(|m| cbor::get_text(m, "cmd")).unwrap_or("").to_string();
        let rsp = Value::Text(cmd.clone());
        let reply = match cmd.as_str() {
            "get-info" => cbor::map([
                ("rsp", rsp),
                ("board", Value::Text(self.o.board.clone())),
                ("logs", Value::Integer(self.seq.into())),
            ]),
            "echo" => cbor::map([
                ("rsp", rsp),
                ("value", req.as_ref().and_then(|m| cbor::get(m, "value")).cloned().unwrap_or(Value::Null)),
            ]),
            _ => cbor::map([("rsp", rsp), ("error", Value::Text("unknown command".into()))]),
        };
        self.send(wire_port(0, &cbor::to_vec(&reply))).await
    }

    async fn command(&mut self, port: u8, line: &str) -> bool {
        let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
        match cmd {
            "help" => {
                self.text(
                    port,
                    "loopback commands: help, echo <text>, reset, burst <n>, big, quiet <secs>, sh, read, plot [stop]\r\n",
                )
                .await
            }
            "echo" => self.text(port, &format!("{arg}\r\n")).await,
            "reset" => {
                self.seq = 0;
                let info = device_info(&self.o.board);
                self.send(info).await && self.send(log(FMT_HELLO, b"loopback reset\0")).await
            }
            "burst" => {
                let n: u32 = arg.parse().unwrap_or(100);
                for _ in 0..n {
                    if !self.one_log().await {
                        return false;
                    }
                }
                self.text(port, &format!("burst of {n} logs\r\n")).await
            }
            "big" => {
                let mut d: Vec<u8> = (0..MAX_PORT_DATA).map(|i| b'a' + (i % 26) as u8).collect();
                let n = d.len();
                d[n - 2..].copy_from_slice(b"\r\n");
                self.send(wire_port(port, &d)).await
            }
            "quiet" => {
                let s: f64 = arg.parse().unwrap_or(3.0);
                let ok = self.text(port, &format!("quiet for {s} s\r\n")).await;
                self.quiet_until = Some(Instant::now() + Duration::from_secs_f64(s));
                ok
            }
            "sh" => {
                self.pending = Some((Pending::TimeOfDay { port }, Instant::now()));
                self.send(wire_port(ports::SEMIHOST, &[uclog_proto::semihost::op::TIMEOFDAY])).await
            }
            "plot" if arg == "stop" => {
                self.plot = None;
                self.text(port, "plots stopped\r\n").await
            }
            "plot" => {
                for m in plot_create() {
                    if !self.send(wire_port(ports::PLOT, &cbor::to_vec(&m))).await {
                        return false;
                    }
                }
                self.plot = Some(0);
                self.text(port, "plots: loopback (lines), spectrum, grid; `plot stop` to stop\r\n").await
            }
            "read" => {
                self.pending = Some((Pending::Read { port }, Instant::now()));
                let mut r = vec![uclog_proto::semihost::op::READ];
                r.extend_from_slice(&0i32.to_le_bytes());
                r.extend_from_slice(&64u32.to_le_bytes());
                self.send(wire_port(ports::SEMIHOST, &r)).await
            }
            _ => self.text(port, &format!("unknown command {cmd:?} (try help)\r\n")).await,
        }
    }

    async fn on_semihost(&mut self, data: &[u8]) -> bool {
        let Some((p, _)) = self.pending.take() else {
            return true; // unsolicited reply: ignored, as by a device
        };
        let i32_at = |o: usize| data.get(o..o + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
        match p {
            Pending::TimeOfDay { port } => {
                let sec = data.get(..8).map(|b| u64::from_le_bytes(b.try_into().unwrap()));
                let ok = self.text(port, &format!("sh: TIMEOFDAY sec={sec:?} ({} bytes)\r\n", data.len())).await;
                let msg = b"hello from loopback\n";
                let mut r = vec![uclog_proto::semihost::op::WRITE];
                r.extend_from_slice(&1u32.to_le_bytes());
                r.extend_from_slice(msg);
                self.pending = Some((Pending::Write { port }, Instant::now()));
                ok && self.send(wire_port(ports::SEMIHOST, &r)).await
            }
            Pending::Write { port } => {
                self.text(port, &format!("sh: WRITE r={:?} errno={:?}\r\n", i32_at(0), i32_at(4))).await
            }
            Pending::Read { port } => {
                let n = i32_at(0).unwrap_or(-1);
                let text = data.get(8..).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
                self.text(port, &format!("read: n={n} errno={:?} {:?}\r\n", i32_at(4), text)).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options() {
        assert_eq!(Options::parse("").unwrap(), Options::default());
        let o = Options::parse("rate=0,board=x").unwrap();
        assert_eq!((o.rate, o.board.as_str()), (0.0, "x"));
        assert!(Options::parse("nope=1").is_err());
    }

    #[tokio::test]
    async fn starts_with_device_info_and_echoes() {
        let mut l = open(LinkSpec::Loopback { opts: "rate=0".into() }, "rate=0").unwrap();
        let LinkEvent::Data(first) = l.rx.recv().await.unwrap() else { panic!() };
        assert_eq!(first, device_info("loopback"));
        l.tx.send(wire_port(61, b"echo hi\r")).await.unwrap();
        loop {
            let LinkEvent::Data(d) = l.rx.recv().await.unwrap() else { panic!() };
            if d == wire_port(61, b"hi\r\n") {
                break;
            }
        }
    }

    #[tokio::test]
    async fn cbor_requests_on_port_0() {
        use cbor::Value;
        let mut l = open(LinkSpec::Loopback { opts: "rate=0".into() }, "rate=0").unwrap();
        let req = cbor::map([("cmd", Value::Text("echo".into())), ("value", Value::Integer(7.into()))]);
        l.tx.send(wire_port(0, &cbor::to_vec(&req))).await.unwrap();
        let want = cbor::map([("rsp", Value::Text("echo".into())), ("value", Value::Integer(7.into()))]);
        loop {
            let LinkEvent::Data(d) = l.rx.recv().await.unwrap() else { panic!() };
            if d == wire_port(0, &cbor::to_vec(&want)) {
                break;
            }
        }
    }
}
