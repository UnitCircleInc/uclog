//! The GUI's async side (GUI plan §2): runs on the tokio runtime, owns the session, and talks to
//! the UI thread over channels. Every event wakes the UI with `request_repaint`.

use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use tokio::sync::mpsc;
use uclog_link::{discover, CloseReason, Device, LinkSpec};
use uclog_session::{Event, FwProgress, SessionHandle};

use crate::settings::Settings;
use crate::{DeviceArgs, LogdataArgs};

/// UI → backend.
#[derive(Debug)]
pub enum Cmd {
    Connect(LinkSpec),
    Disconnect,
    Send(u8, Vec<u8>),
    Stdin(Vec<u8>),
    /// Load a logdata file; with the device's app_hash, also store it in the cache under that
    /// hash so the device finds it next time.
    LoadLogdata(PathBuf, Option<Vec<u8>>),
    FwStart(PathBuf),
    FwCancel,
    RefreshDevices,
    BleScan,
    /// The settings changed (resolver, files directory, USB filters).
    Settings(Box<Settings>),
}

/// Backend → UI.
#[derive(Debug)]
pub enum Ev {
    Session(Event),
    Connecting(String),
    Connected {
        uri: String,
        description: String,
        hub: bool,
    },
    /// The message, and on Linux when access was refused, how to get it.
    ConnectFailed(String, Option<&'static [&'static str]>),
    /// Reconnecting in a second (a direct device went away).
    Reconnecting,
    Devices(Vec<Device>),
    BleDevices(Result<Vec<Device>, String>),
    Fw(FwProgress),
    FwDone(Result<String, String>),
    Logdata(Result<String, String>),
}

#[derive(Clone)]
pub struct Backend {
    tx: mpsc::UnboundedSender<Cmd>,
}

impl Backend {
    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
}

struct Ui {
    tx: std_mpsc::Sender<Ev>,
    ctx: egui::Context,
}

impl Ui {
    fn send(&self, e: Ev) {
        let _ = self.tx.send(e);
        self.ctx.request_repaint();
    }
}

struct Runner {
    ui: Ui,
    device: DeviceArgs,
    logdata: LogdataArgs,
    files_dir: Option<PathBuf>,
    settings: Settings,
    session: Option<SessionHandle>,
    events: Option<mpsc::Receiver<Event>>,
    new_session: mpsc::Sender<(SessionHandle, mpsc::Receiver<Event>)>,
    spec: Option<LinkSpec>,
    reconnect: bool,
    fw_task: Option<tokio::task::JoinHandle<()>>,
}

/// Start the backend on the current tokio runtime.
pub fn start(
    ctx: egui::Context,
    device: DeviceArgs,
    logdata: LogdataArgs,
    files_dir: Option<PathBuf>,
    settings: Settings,
) -> (Backend, std_mpsc::Receiver<Ev>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let (ui_tx, ui_rx) = std_mpsc::channel();
    let (ns_tx, ns_rx) = mpsc::channel(2);
    let r = Runner {
        ui: Ui { tx: ui_tx, ctx },
        device,
        logdata,
        files_dir,
        settings,
        session: None,
        events: None,
        new_session: ns_tx,
        spec: None,
        reconnect: false,
        fw_task: None,
    };
    tokio::spawn(r.run(rx, ns_rx));
    (Backend { tx }, ui_rx)
}

impl Runner {
    fn connect(&mut self, spec: LinkSpec) {
        self.disconnect();
        self.spec = Some(spec.clone());
        self.reconnect = true;
        let cfg = crate::front::session_config(&self.logdata, self.files_dir.clone(), &self.settings);
        let spec = spec.with_baud(self.device.baud);
        let tx = self.ui.tx.clone();
        let ctx = self.ui.ctx.clone();
        let ns = self.new_session.clone();
        tokio::spawn(async move {
            let ui = Ui { tx, ctx };
            ui.send(Ev::Connecting(spec.to_string()));
            match uclog_link::open(&spec).await {
                Ok(link) => {
                    let description = link.description.clone();
                    let (h, ev) = uclog_session::start(link, cfg);
                    ui.send(Ev::Connected { uri: spec.to_string(), description, hub: spec.is_hub() });
                    let _ = ns.send((h, ev)).await;
                }
                Err(e) => ui.send(Ev::ConnectFailed(format!("{spec}: {e}"), e.access_hint())),
            }
        });
    }

    fn disconnect(&mut self) {
        if let Some(h) = self.session.take() {
            tokio::spawn(async move { h.close().await });
        }
        self.events = None;
        if let Some(t) = self.fw_task.take() {
            t.abort();
        }
    }

    async fn run(
        mut self,
        mut cmds: mpsc::UnboundedReceiver<Cmd>,
        mut new_session: mpsc::Receiver<(SessionHandle, mpsc::Receiver<Event>)>,
    ) {
        // Device selection: an explicit device connects; `auto` connects when exactly one
        // matches, otherwise the UI opens the Connect dialog (it asks for the device list).
        match self.device.device.clone() {
            LinkSpec::Auto => {
                let filters = crate::front::usb_filters(&self.device.usb_filter, &self.settings);
                match tokio::task::spawn_blocking(move || discover::resolve_auto(&filters)).await {
                    Ok(Ok(spec)) => self.connect(spec),
                    Ok(Err((msg, _))) => self.ui.send(Ev::ConnectFailed(msg, None)),
                    Err(_) => {}
                }
            }
            spec => self.connect(spec),
        }
        let mut reconnect_at: Option<tokio::time::Instant> = None;
        let mut tick = tokio::time::interval(Duration::from_millis(200));
        loop {
            tokio::select! {
                c = cmds.recv() => match c {
                    Some(c) => self.on_cmd(c).await,
                    None => break,
                },
                e = async { self.events.as_mut().unwrap().recv().await }, if self.events.is_some() => {
                    let Some(e) = e else { self.events = None; continue };
                    let closed = matches!(&e, Event::Closed(_));
                    let detached = matches!(&e, Event::Closed(CloseReason::Detached(_) | CloseReason::Eof | CloseReason::Error(_)));
                    let _ = self.ui.tx.send(Ev::Session(e));
                    // Batch what is queued before waking the UI.
                    if !closed {
                        while let Some(Ok(e)) = self.events.as_mut().map(|r| r.try_recv()) {
                            let c = matches!(&e, Event::Closed(_));
                            let _ = self.ui.tx.send(Ev::Session(e));
                            if c {
                                self.session = None;
                                self.events = None;
                                break;
                            }
                        }
                    }
                    self.ui.ctx.request_repaint();
                    if closed {
                        self.session = None;
                        self.events = None;
                        if detached && self.reconnect && !self.spec.as_ref().is_some_and(LinkSpec::is_hub) {
                            reconnect_at = Some(tokio::time::Instant::now() + Duration::from_secs(1));
                            self.ui.send(Ev::Reconnecting);
                        }
                    }
                }
                s = new_session.recv() => if let Some((h, ev)) = s {
                    self.session = Some(h);
                    self.events = Some(ev);
                    reconnect_at = None;
                },
                _ = tick.tick() => {
                    if reconnect_at.is_some_and(|t| tokio::time::Instant::now() >= t) {
                        reconnect_at = None;
                        if let (Some(s), None) = (self.spec.clone(), &self.session) {
                            self.connect(s);
                        }
                    }
                }
            }
        }
        self.disconnect();
    }

    async fn on_cmd(&mut self, c: Cmd) {
        match c {
            Cmd::Connect(spec) => self.connect(spec),
            Cmd::Disconnect => {
                self.reconnect = false;
                self.disconnect();
                self.ui.send(Ev::Session(Event::Closed(CloseReason::Error("disconnected".into()))));
            }
            Cmd::Send(port, data) => {
                if let Some(h) = &self.session {
                    let _ = h.send(port, data).await;
                }
            }
            Cmd::Stdin(d) => {
                if let Some(h) = &self.session {
                    let _ = h.stdin(d).await;
                }
            }
            Cmd::LoadLogdata(p, app_hash) => {
                let cached = app_hash.and_then(|h| remember(&self.logdata, &p, &h));
                match &self.session {
                    Some(h) => {
                        let r = h.load_logdata(p.clone()).await.map(|t| {
                            let mut s = format!("logdata for target {t}: {}", p.display());
                            if let Some(c) = cached {
                                s += &format!(" (remembered as {})", c.display());
                            }
                            s
                        });
                        self.ui.send(Ev::Logdata(r));
                    }
                    None => {
                        self.logdata.logdata.push(p.clone());
                        self.ui.send(Ev::Logdata(Ok(format!("{} will be used when connected", p.display()))));
                    }
                }
            }
            Cmd::FwStart(p) => {
                let Some(h) = self.session.clone() else {
                    self.ui.send(Ev::FwDone(Err("not connected".into())));
                    return;
                };
                let (tx, ctx) = (self.ui.tx.clone(), self.ui.ctx.clone());
                self.fw_task = Some(tokio::spawn(async move {
                    let ui = Ui { tx, ctx };
                    let image = match std::fs::read(&p) {
                        Ok(d) => d,
                        Err(e) => return ui.send(Ev::FwDone(Err(format!("{}: {e}", p.display())))),
                    };
                    let r = h.fw_update(&image, |pr| ui.send(Ev::Fw(pr))).await;
                    let r = match r {
                        Ok(port) => {
                            h.add_image(p.clone()).await;
                            Ok(format!("Update scheduled on port {port}; the device installs it at its next reset."))
                        }
                        Err(e) => Err(e.to_string()),
                    };
                    ui.send(Ev::FwDone(r));
                }));
            }
            Cmd::FwCancel => {
                if let Some(t) = self.fw_task.take() {
                    t.abort();
                    self.ui.send(Ev::FwDone(Err("cancelled".into())));
                }
            }
            Cmd::RefreshDevices => {
                let filters = crate::front::usb_filters(&self.device.usb_filter, &self.settings);
                let (tx, ctx) = (self.ui.tx.clone(), self.ui.ctx.clone());
                tokio::spawn(async move {
                    let ui = Ui { tx, ctx };
                    let mut v = tokio::task::spawn_blocking(move || discover::local_devices(&filters))
                        .await
                        .unwrap_or_default();
                    v.extend(crate::front::local_hub_devices());
                    let names: Vec<String> = v.iter().map(|d| d.name.clone()).collect();
                    ui.send(Ev::Devices(v.clone()));
                    if let Ok(h) = discover::hubs(Duration::from_secs(1)).await {
                        v.extend(h.into_iter().filter(|d| !names.contains(&d.name)));
                        ui.send(Ev::Devices(v));
                    }
                });
            }
            Cmd::BleScan => {
                let (tx, ctx) = (self.ui.tx.clone(), self.ui.ctx.clone());
                tokio::spawn(async move {
                    let r = discover::ble_devices(Duration::from_secs(5)).await.map_err(|e| e.to_string());
                    Ui { tx, ctx }.send(Ev::BleDevices(r));
                });
            }
            Cmd::Settings(s) => {
                if self.files_dir.is_none() && s.files_dir != self.settings.files_dir {
                    if let Some(h) = &self.session {
                        h.set_files_dir(s.files_dir.clone()).await;
                    }
                }
                self.settings = *s;
            }
        }
    }
}

/// Store a manually chosen logdata file in the cache as `<app_hash>.logdata` (GUI plan §3:
/// per-device memory of manual selections). Returns the cache path.
fn remember(l: &LogdataArgs, p: &std::path::Path, app_hash: &[u8]) -> Option<PathBuf> {
    let dir = l.cache_dir.clone().or_else(crate::default_cache_dir)?;
    let data = std::fs::read(p).ok()?;
    let cbor = if data.starts_with(b"\x7fELF") { uclog_logdata::LogData::cbor_from_elf(&data).ok()? } else { data };
    uclog_logdata::LogData::from_cbor(&cbor, "").ok()?;
    let dst = dir.join(format!("{}.logdata", hex::encode(app_hash)));
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(&dst, cbor).ok()?;
    Some(dst)
}
