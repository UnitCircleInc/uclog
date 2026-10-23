//! `uclog tui` (docs/plans/05-tui.md): the terminal UI, plus the `--plain` / `--json` streaming
//! modes used when stdout is not a terminal.

pub mod app;
pub mod lineedit;
pub use crate::front::logs;
pub mod picker;
pub mod plotcsv;
pub mod ui;

#[cfg(test)]
mod tests_ui;

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context;
use clap::Args;
use crossterm::event::{Event as TEvent, EventStream, KeyEventKind};
use crossterm::{execute, terminal};
use futures_lite::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;
use uclog_link::{discover, CloseReason, LinkSpec};
use uclog_session::{Event, SessionHandle};

use self::app::{Action, App, Bg};
use self::logs::{level_num, Row};
use self::plotcsv::PlotCsv;
use crate::settings::{self, Settings};
use crate::{DeviceArgs, LogdataArgs};

#[derive(Args)]
pub struct TuiArgs {
    #[command(flatten)]
    device: DeviceArgs,
    #[command(flatten)]
    logdata: LogdataArgs,
    /// Semihost sandbox directory [default: from settings]
    #[arg(long, value_name = "DIR")]
    files_dir: Option<PathBuf>,
    /// Stream decoded lines (ts:level:file:line:text) instead of the UI; automatic when stdout is not a terminal
    #[arg(long, conflicts_with = "json")]
    plain: bool,
    /// Stream one JSON record per line instead of the UI
    #[arg(long)]
    json: bool,
    /// Lowest level shown (DEBUG, INFO, WARN, ERROR, FATAL)
    #[arg(long, value_name = "LEVEL")]
    level: Option<String>,
    /// Write port-60 plot data to CSV files in this directory
    #[arg(long, value_name = "DIR")]
    plot_csv: Option<PathBuf>,
    /// Settings file [default: the platform config directory]
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
}

fn session_config(a: &TuiArgs, s: &Settings) -> uclog_session::Config {
    crate::front::session_config(&a.logdata, a.files_dir.clone(), s)
}

fn usb_filters(a: &TuiArgs, s: &Settings) -> Vec<discover::UsbFilter> {
    crate::front::usb_filters(&a.device.usb_filter, s)
}

pub async fn run(a: TuiArgs) -> anyhow::Result<ExitCode> {
    let (settings, settings_path, note) = Settings::load(a.config.as_deref());
    let stream = a.plain || a.json || !std::io::stdout().is_terminal();
    if stream {
        if let Some(n) = note {
            eprintln!("uclog: {n}");
        }
        return run_stream(a, settings).await;
    }
    run_ui(a, settings, settings_path, note).await
}

// ---------------------------------------------------------------------------------------------
// Streaming modes

async fn run_stream(a: TuiArgs, s: Settings) -> anyhow::Result<ExitCode> {
    let mut d = a.device.clone();
    d.usb_filter = usb_filters(&a, &s);
    let link = crate::cmd::open(&d).await?;
    let (h, mut ev) = uclog_session::start(link, session_config(&a, &s));
    let min = a.level.as_deref().map(level_num).unwrap_or(0);
    let mut plots = a.plot_csv.clone().map(PlotCsv::new).transpose().context("--plot-csv")?;
    let mut seq = 0;
    let mut out = std::io::stdout().lock();
    loop {
        let e = tokio::select! {
            e = ev.recv() => match e { Some(e) => e, None => break },
            _ = tokio::signal::ctrl_c() => break,
        };
        match e {
            Event::Log(r) => {
                seq += 1;
                let row = Row::from_record(seq, &r);
                if r.error.is_some() {
                    eprintln!("{}", row.plain());
                } else if row.level >= min {
                    let line = if a.json {
                        serde_json::json!({"ts": r.ts, "target": r.target, "level": r.level, "file": r.file,
                                           "line": r.line, "text": r.text})
                        .to_string()
                    } else {
                        row.plain()
                    };
                    if writeln!(out, "{line}").is_err() {
                        break; // stdout closed (e.g. piped into head)
                    }
                }
            }
            Event::Port { port: uclog_proto::ports::PLOT, data } => {
                if let Some(p) = &mut plots {
                    p.handle(&data);
                }
            }
            Event::Console(c) => {
                let _ = std::io::stderr().write_all(&c.data);
            }
            Event::StdinWanted => {
                // No terminal input in these modes: answer EOF.
                let _ = h.stdin(vec![]).await;
            }
            Event::Note(n) => eprintln!("uclog: {n}"),
            Event::Logdata { source: Some(s), .. } => eprintln!("uclog: logdata: {s}"),
            Event::Liveness(uclog_session::Liveness::NotResponding) => eprintln!("uclog: device not responding"),
            Event::Closed(r) => {
                eprintln!("uclog: {r}");
                return Ok(ExitCode::FAILURE);
            }
            _ => {}
        }
    }
    h.close().await;
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------------------------
// Interactive UI

/// Restores the terminal even on panic or early return.
struct TermGuard;

impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(std::io::stdout(), terminal::LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

struct Runner {
    args: TuiArgs,
    session: Option<SessionHandle>,
    events: Option<mpsc::Receiver<Event>>,
    bg: mpsc::Sender<Bg>,
    /// Session handed over by a finished connect task.
    new_session: mpsc::Sender<(SessionHandle, mpsc::Receiver<Event>)>,
    spec: Option<LinkSpec>,
    /// Reconnect after the device went away (not after Disconnect).
    reconnect: bool,
    fw_task: Option<tokio::task::JoinHandle<()>>,
    plots: Option<PlotCsv>,
    settings_path: Option<PathBuf>,
}

impl Runner {
    fn connect(&mut self, app: &App, spec: LinkSpec) {
        self.disconnect();
        self.spec = Some(spec.clone());
        self.reconnect = true;
        let cfg = session_config(&self.args, &app.settings);
        let bg = self.bg.clone();
        let ns = self.new_session.clone();
        let spec = spec.with_baud(self.args.device.baud);
        tokio::spawn(async move {
            let _ = bg.send(Bg::Connecting(spec.to_string())).await;
            match uclog_link::open(&spec).await {
                Ok(link) => {
                    let description = link.description.clone();
                    let (h, ev) = uclog_session::start(link, cfg);
                    let _ = bg.send(Bg::Connected { uri: spec.to_string(), description }).await;
                    let _ = ns.send((h, ev)).await;
                }
                Err(e) => {
                    let _ = bg.send(Bg::ConnectFailed(format!("{spec}: {e}"))).await;
                }
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

    async fn act(&mut self, app: &mut App, a: Action) -> bool {
        match a {
            Action::Quit => return false,
            Action::Connect(spec) => {
                app.modal = app::Modal::None;
                self.connect(app, spec);
            }
            Action::Disconnect => {
                self.reconnect = false;
                self.disconnect();
                app.on_session(Event::Closed(CloseReason::Error("disconnected".into())));
                app.modal = app::Modal::None;
            }
            Action::Send(port, data) => match &self.session {
                Some(h) => {
                    let _ = h.send(port, data).await;
                }
                None => app.say("not connected (F2 to connect)"),
            },
            Action::Stdin(d) => {
                if let Some(h) = &self.session {
                    let _ = h.stdin(d).await;
                }
            }
            Action::LoadLogdata(p) => match &self.session {
                Some(h) => match h.load_logdata(p.clone()).await {
                    Ok(t) => {
                        app.device.logdata = Some(format!("manual {}", p.display()));
                        if !app.device.targets.contains(&t) {
                            app.device.targets.push(t);
                        }
                        app.say(format!("logdata for target {t}: {}", p.display()));
                    }
                    Err(e) => app.say(e),
                },
                None => {
                    // Not connected: use it for the next connection.
                    self.args.logdata.logdata.push(p.clone());
                    app.say(format!("{} will be used when connected", p.display()));
                }
            },
            Action::FwStart(p) => {
                let Some(h) = self.session.clone() else {
                    app.on_bg(Bg::FwDone(Err("not connected".into())));
                    return true;
                };
                let bg = self.bg.clone();
                self.fw_task = Some(tokio::spawn(async move {
                    let image = match std::fs::read(&p) {
                        Ok(d) => d,
                        Err(e) => {
                            let _ = bg.send(Bg::FwDone(Err(format!("{}: {e}", p.display())))).await;
                            return;
                        }
                    };
                    let b2 = bg.clone();
                    let r = h
                        .fw_update(&image, move |pr| {
                            let _ = b2.try_send(Bg::Fw(pr));
                        })
                        .await;
                    let r = match r {
                        Ok(port) => {
                            h.add_image(p.clone()).await;
                            Ok(format!("Update scheduled on port {port}; the device installs it at its next reset."))
                        }
                        Err(e) => Err(e.to_string()),
                    };
                    let _ = bg.send(Bg::FwDone(r)).await;
                }));
            }
            Action::FwCancel => {
                if let Some(t) = self.fw_task.take() {
                    t.abort();
                }
            }
            Action::RefreshDevices => {
                let bg = self.bg.clone();
                let filters = usb_filters(&self.args, &app.settings);
                tokio::spawn(async move {
                    let mut v = tokio::task::spawn_blocking(move || discover::local_devices(&filters))
                        .await
                        .unwrap_or_default();
                    v.extend(crate::front::local_hub_devices());
                    let names: Vec<String> = v.iter().map(|d| d.name.clone()).collect();
                    let _ = bg.send(Bg::Devices(v.clone())).await;
                    if let Ok(h) = discover::hubs(Duration::from_secs(1)).await {
                        v.extend(h.into_iter().filter(|d| !names.contains(&d.name)));
                        let _ = bg.send(Bg::Devices(v)).await;
                    }
                });
            }
            Action::BleScan => {
                let bg = self.bg.clone();
                tokio::spawn(async move {
                    let r = discover::ble_devices(Duration::from_secs(5)).await.map_err(|e| e.to_string());
                    let _ = bg.send(Bg::BleDevices(r)).await;
                });
            }
            Action::SaveSettings { token } => {
                let mut msg = vec![];
                if let Some(t) = token {
                    match settings::set_keyring_token(t.as_deref()) {
                        Ok(()) => msg.push("token saved in the keyring".to_string()),
                        Err(e) => msg.push(format!("keyring: {e}")),
                    }
                }
                match &self.settings_path {
                    Some(p) => match app.settings.save(p) {
                        Ok(()) => msg.push(format!("saved {}", p.display())),
                        Err(e) => msg.push(format!("{}: {e}", p.display())),
                    },
                    None => msg.push("no settings directory".into()),
                }
                app.say(msg.join("; "));
            }
            Action::SetFilesDir(d) => {
                self.args.files_dir = d.clone();
                if let Some(h) = &self.session {
                    h.set_files_dir(d).await;
                }
            }
            Action::Copy(text) => {
                copy(&text);
                app.say("copied");
            }
            Action::Export { name, content } => {
                let dir = app.settings.export_dir.clone().unwrap_or_else(|| app.cwd.clone());
                let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, "txt"));
                let path = dir.join(format!("uclog-{stem}-{}.{ext}", crate::front::timestamp()));
                match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, content)) {
                    Ok(()) => app.say(format!("exported {}", path.display())),
                    Err(e) => app.say(format!("{}: {e}", path.display())),
                }
            }
        }
        true
    }
}

/// Clipboard: OSC 52 (works over SSH in most terminals), plus the OS clipboard when available.
fn copy(text: &str) {
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = out.flush();
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(text.to_string());
    }
}

fn base64(d: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

async fn run_ui(
    a: TuiArgs,
    settings: Settings,
    settings_path: Option<PathBuf>,
    note: Option<String>,
) -> anyhow::Result<ExitCode> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut app = App::new(settings, cwd);
    app.token_set = settings::keyring_token().is_some();
    if let Some(l) = &a.level {
        let mut f = app.logs.filter.clone();
        f.min_level = level_num(l);
        app.logs.set_filter(f);
    }
    if let Some(n) = note {
        app.say(n);
    }
    let plots = a.plot_csv.clone().map(PlotCsv::new).transpose().context("--plot-csv")?;
    let (bg_tx, mut bg_rx) = mpsc::channel(64);
    let (ns_tx, mut ns_rx) = mpsc::channel(2);
    let mut r = Runner {
        args: a,
        session: None,
        events: None,
        bg: bg_tx,
        new_session: ns_tx,
        spec: None,
        reconnect: false,
        fw_task: None,
        plots,
        settings_path,
    };

    // Device selection: an explicit device connects; `auto` connects when exactly one matches,
    // otherwise the Connect list opens.
    match &r.args.device.device {
        LinkSpec::Auto => match discover::resolve_auto(&usb_filters(&r.args, &app.settings)) {
            Ok(spec) => r.connect(&app, spec),
            Err((msg, _)) => {
                app.say(msg);
                app.modal = app::Modal::Connect;
                r.act(&mut app, Action::RefreshDevices).await;
            }
        },
        spec => {
            let s = spec.clone();
            r.connect(&app, s);
        }
    }

    terminal::enable_raw_mode()?;
    execute!(std::io::stdout(), terminal::EnterAlternateScreen)?;
    let _guard = TermGuard;
    let mut term = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let mut keys = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut dirty = true;
    let mut reconnect_at: Option<tokio::time::Instant> = None;
    loop {
        tokio::select! {
            k = keys.next() => match k {
                Some(Ok(TEvent::Key(k))) if k.kind != KeyEventKind::Release => {
                    for a in app.on_key(k) {
                        if !r.act(&mut app, a).await {
                            r.disconnect();
                            let _ = app.settings.save_quietly(r.settings_path.as_deref());
                            return Ok(ExitCode::SUCCESS);
                        }
                    }
                    dirty = true;
                }
                Some(Ok(TEvent::Resize(..))) => dirty = true,
                Some(Err(_)) | None => break,
                _ => {}
            },
            e = async { r.events.as_mut().unwrap().recv().await }, if r.events.is_some() => {
                let mut batch = vec![];
                match e {
                    Some(e) => batch.push(e),
                    None => { r.events = None; continue; }
                }
                // Drain what is queued, so high log rates do not redraw per line.
                while batch.len() < 2000 {
                    match r.events.as_mut().unwrap().try_recv() {
                        Ok(e) => batch.push(e),
                        Err(_) => break,
                    }
                }
                for e in batch {
                    if let Event::Port { port: uclog_proto::ports::PLOT, data } = &e {
                        if let Some(p) = &mut r.plots { p.handle(data); }
                    }
                    let closed = matches!(e, Event::Closed(_));
                    let detached = matches!(&e, Event::Closed(CloseReason::Detached(_) | CloseReason::Eof | CloseReason::Error(_)));
                    for a in app.on_session(e) {
                        r.act(&mut app, a).await;
                    }
                    if closed {
                        r.session = None;
                        r.events = None;
                        if detached && r.reconnect && !r.spec.as_ref().is_some_and(LinkSpec::is_hub) {
                            reconnect_at = Some(tokio::time::Instant::now() + Duration::from_secs(1));
                        }
                        break;
                    }
                }
                dirty = true;
            }
            s = ns_rx.recv() => if let Some((h, ev)) = s {
                r.session = Some(h);
                r.events = Some(ev);
                reconnect_at = None;
                dirty = true;
            },
            b = bg_rx.recv() => if let Some(b) = b {
                app.on_bg(b);
                dirty = true;
            },
            _ = tick.tick() => {
                if reconnect_at.is_some_and(|t| tokio::time::Instant::now() >= t) {
                    reconnect_at = None;
                    if let Some(s) = r.spec.clone() {
                        if r.session.is_none() { r.connect(&app, s); }
                    }
                }
                // Status messages expire.
                if app.status.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(6)) {
                    app.status = None;
                    dirty = true;
                }
                if dirty {
                    term.draw(|f| ui::draw(f, &app))?;
                    dirty = false;
                }
            }
        }
    }
    r.disconnect();
    Ok(ExitCode::SUCCESS)
}

impl Settings {
    /// Save on exit (terminal history); errors are ignored.
    fn save_quietly(&self, p: Option<&std::path::Path>) -> std::io::Result<()> {
        match p {
            Some(p) => self.save(p),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_timestamp() {
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        assert_eq!(base64(b"hi!"), "aGkh");
        assert_eq!(base64(b"a"), "YQ==");
        let t = crate::front::timestamp();
        assert_eq!(t.len(), 15);
        assert!(t.starts_with("20"));
    }
}
