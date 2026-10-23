//! GUI state and the main frame: menus, status bar, the dock of views, and dialogs.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use egui::{Color32, RichText};
use egui_dock::{DockArea, DockState, NodeIndex, Style as DockStyle};
use serde::{Deserialize, Serialize};
use uclog_link::{CloseReason, Device};
use uclog_proto::cbor::{self, Value};
use uclog_proto::ports;
use uclog_session::{Event, FwProgress, Liveness};

use super::backend::{Backend, Cmd, Ev};
use super::plots::PlotSet;
use super::term::Terminal;
use super::views;
use crate::front::logs::{Filter, LogStore};
use crate::settings::{self, Settings};

pub const RESPONSES_MAX: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    Logs,
    Terminal,
    Commands,
    Device,
    Fw,
    Plot(String),
}

impl Tab {
    pub fn title(&self) -> String {
        match self {
            Tab::Logs => "Logs".into(),
            Tab::Terminal => "Terminal".into(),
            Tab::Commands => "Commands".into(),
            Tab::Device => "Device".into(),
            Tab::Fw => "FW Update".into(),
            Tab::Plot(n) => format!("📈 {n}"),
        }
    }
}

/// Add a tab to the first leaf without making it the active one (a new plot must not hide the
/// view the user is looking at).
pub fn add_in_background(dock: &mut DockState<Tab>, tab: Tab) {
    if let Some((_, leaf)) = dock.iter_leaves_mut().next() {
        leaf.tabs.push(tab);
        return;
    }
    dock.push_to_first_leaf(tab);
}

/// Logs (left, large) with Terminal and Commands below; Device and FW as tabs next to Logs.
pub fn default_layout() -> DockState<Tab> {
    let mut d = DockState::new(vec![Tab::Logs, Tab::Device, Tab::Fw]);
    let s = d.main_surface_mut();
    s.split_below(NodeIndex::root(), 0.65, vec![Tab::Terminal, Tab::Commands]);
    d
}

#[derive(Default)]
pub struct Conn {
    pub connecting: Option<String>,
    pub uri: Option<String>,
    pub description: String,
    pub hub: bool,
    pub liveness: Option<Liveness>,
    pub closed: Option<String>,
    pub busy: bool,
    pub reconnecting: bool,
}

impl Conn {
    pub fn connected(&self) -> bool {
        self.uri.is_some() && self.closed.is_none()
    }

    /// Status text and colour.
    pub fn status(&self) -> (String, Color32) {
        if let Some(u) = &self.connecting {
            return (format!("connecting to {u}…"), Color32::from_rgb(220, 180, 60));
        }
        if self.busy {
            return ("hub busy (another client is connected)".into(), Color32::from_rgb(230, 90, 80));
        }
        if let Some(c) = &self.closed {
            let t = if self.reconnecting { format!("{c}; reconnecting…") } else { c.clone() };
            return (t, Color32::from_rgb(230, 90, 80));
        }
        match (&self.uri, self.liveness) {
            (None, _) => ("not connected".into(), Color32::GRAY),
            (Some(u), Some(Liveness::Alive)) => (format!("connected: {u}"), Color32::from_rgb(90, 190, 110)),
            (Some(u), Some(Liveness::NotResponding)) => {
                (format!("{u}: device not responding"), Color32::from_rgb(230, 150, 60))
            }
            (Some(u), _) if self.hub => {
                (format!("{u}: waiting for hub approval or device data…"), Color32::from_rgb(220, 180, 60))
            }
            (Some(u), _) => (format!("{u}: waiting for the device…"), Color32::from_rgb(220, 180, 60)),
        }
    }
}

#[derive(Default)]
pub struct DeviceView {
    pub info: Option<Value>,
    pub app_hash: Option<Vec<u8>>,
    pub logdata: Option<String>,
    pub targets: Vec<u8>,
}

pub struct LogView {
    pub level: u8,
    pub source: String,
    pub line: String,
    pub search: String,
    pub search_error: Option<String>,
    pub follow: bool,
    pub selected: BTreeSet<u64>,
    /// Anchor for shift-click selection (visible index).
    pub anchor: Option<usize>,
}

impl Default for LogView {
    fn default() -> Self {
        LogView {
            level: 0,
            source: String::new(),
            line: String::new(),
            search: String::new(),
            search_error: None,
            follow: true,
            selected: BTreeSet::new(),
            anchor: None,
        }
    }
}

#[derive(Default)]
pub struct TermView {
    pub input: String,
    /// Position while browsing history (`None` = editing a new line).
    pub hist: Option<usize>,
    pub stdin_wanted: bool,
}

#[derive(Default)]
pub struct CmdView {
    pub responses: VecDeque<String>,
    /// Answers for `$arg` fields, by (button label, key, placeholder).
    pub args: HashMap<(String, String, String), String>,
    pub free: String,
}

#[derive(Default)]
pub struct FwView {
    pub path: String,
    pub suggestions: Vec<PathBuf>,
    pub running: bool,
    pub progress: Option<FwProgress>,
    pub result: Option<Result<String, String>>,
}

#[derive(Default)]
pub struct ConnectDialog {
    pub open: bool,
    pub devices: Vec<Device>,
    pub ble: Vec<Device>,
    pub ble_scanning: bool,
    pub ble_error: Option<String>,
    pub filter: String,
    pub address: String,
    pub selected: Option<String>,
}

pub struct SettingsDialog {
    pub open: bool,
    pub s: Settings,
    pub token: String,
    pub token_set: bool,
    pub usb: String,
    pub on_connect: String,
    pub term_buttons: String,
    pub cmd_buttons: String,
    pub error: Option<String>,
}

/// One-shot actions requested from the command line (screenshots for docs and tests).
#[derive(Default)]
pub struct Script {
    /// Views to bring to the front just before the screenshot (`Logs`, a plot name, …).
    pub show: Vec<String>,
    pub screenshot: Option<PathBuf>,
    pub at: Option<Instant>,
    pub exit: bool,
    pub requested: bool,
    show_done: bool,
}

pub struct GuiApp {
    pub settings: Settings,
    pub settings_path: Option<PathBuf>,
    pub backend: Backend,
    rx: mpsc::Receiver<Ev>,
    pub cwd: PathBuf,
    pub conn: Conn,
    pub logs: LogStore,
    pub log_view: LogView,
    pub term: Terminal,
    pub term_view: TermView,
    pub cmds: CmdView,
    pub plots: PlotSet,
    pub plot_views: HashMap<String, views::plots::PlotView>,
    pub device: DeviceView,
    pub fw: FwView,
    pub dock: Option<DockState<Tab>>,
    pub connect: ConnectDialog,
    pub settings_dlg: Option<SettingsDialog>,
    pub show_about: bool,
    pub status: Option<(String, Instant)>,
    pub port_counts: BTreeMap<u8, u64>,
    pub notes: VecDeque<String>,
    pub script: Script,
    pub quit: bool,
    sent_on_connect: bool,
    applied_scale: f32,
    applied_theme: String,
}

impl GuiApp {
    pub fn new(
        settings: Settings,
        settings_path: Option<PathBuf>,
        backend: Backend,
        rx: mpsc::Receiver<Ev>,
        dock: Option<DockState<Tab>>,
    ) -> Self {
        let max = settings.max_log_lines;
        let cwd = std::env::current_dir().unwrap_or_default();
        GuiApp {
            settings,
            settings_path,
            backend,
            rx,
            fw: FwView { suggestions: crate::front::signed_images(&cwd, 4), ..Default::default() },
            cwd,
            conn: Conn::default(),
            logs: LogStore::new(max),
            log_view: LogView::default(),
            term: Terminal::default(),
            term_view: TermView::default(),
            cmds: CmdView::default(),
            plots: PlotSet::default(),
            plot_views: HashMap::new(),
            device: DeviceView::default(),
            dock: Some(dock.unwrap_or_else(default_layout)),
            connect: ConnectDialog::default(),
            settings_dlg: None,
            show_about: false,
            status: None,
            port_counts: BTreeMap::new(),
            notes: VecDeque::new(),
            script: Script::default(),
            quit: false,
            sent_on_connect: false,
            applied_scale: 0.0,
            applied_theme: String::new(),
        }
    }

    pub fn say(&mut self, s: impl Into<String>) {
        let s = s.into();
        self.note(s.clone());
        self.status = Some((s, Instant::now()));
    }

    fn note(&mut self, s: String) {
        self.notes.push_back(s);
        while self.notes.len() > 200 {
            self.notes.pop_front();
        }
    }

    pub fn send(&self, port: u8, data: Vec<u8>) {
        self.backend.send(Cmd::Send(port, data));
    }

    pub fn open_connect(&mut self) {
        self.connect.open = true;
        self.backend.send(Cmd::RefreshDevices);
    }

    /// Show a view: focus its tab, or add it back if it was closed.
    pub fn focus(&mut self, tab: Tab) {
        let Some(dock) = &mut self.dock else { return };
        match dock.find_tab(&tab) {
            Some(i) => {
                let _ = dock.set_active_tab(i);
            }
            None => dock.push_to_focused_leaf(tab),
        }
    }

    pub fn save_settings(&mut self) {
        if let Some(p) = &self.settings_path {
            if let Err(e) = self.settings.save(p) {
                self.say(format!("{}: {e}", p.display()));
            }
        }
    }

    pub fn set_filter(&mut self) {
        let re = if self.log_view.search.is_empty() {
            Ok(None)
        } else {
            regex::RegexBuilder::new(&self.log_view.search).case_insensitive(true).build().map(Some)
        };
        match re {
            Ok(search) => {
                self.log_view.search_error = None;
                self.logs.set_filter(Filter {
                    min_level: self.log_view.level,
                    source: self.log_view.source.clone(),
                    line: self.log_view.line.clone(),
                    search,
                });
            }
            Err(e) => self.log_view.search_error = Some(e.to_string()),
        }
    }

    // ------------------------------------------------------------------------------- events

    /// Apply what the backend sent. Bounded per frame so a flood cannot freeze the UI.
    pub fn drain(&mut self) {
        let mut n = 0;
        while n < 50_000 {
            match self.rx.try_recv() {
                Ok(e) => self.on_ev(e),
                Err(_) => break,
            }
            n += 1;
        }
        if n == 50_000 {
            // More is waiting: come back next frame.
            self.status = Some(("busy: catching up with the device".into(), Instant::now()));
        }
    }

    pub fn on_ev(&mut self, e: Ev) {
        match e {
            Ev::Session(e) => self.on_session(e),
            Ev::Connecting(u) => {
                self.conn = Conn { connecting: Some(u), ..Default::default() };
                self.sent_on_connect = false;
            }
            Ev::Connected { uri, description, hub } => {
                self.conn = Conn { uri: Some(uri), description, hub, ..Default::default() };
                self.connect.open = false;
            }
            Ev::ConnectFailed(m) => {
                self.conn.connecting = None;
                self.say(m);
                self.open_connect();
            }
            Ev::Reconnecting => self.conn.reconnecting = true,
            Ev::Devices(v) => self.connect.devices = v,
            Ev::BleDevices(r) => {
                self.connect.ble_scanning = false;
                match r {
                    Ok(v) => {
                        self.connect.ble = v;
                        self.connect.ble_error = None;
                    }
                    Err(e) => self.connect.ble_error = Some(e),
                }
            }
            Ev::Fw(p) => self.fw.progress = Some(p),
            Ev::FwDone(r) => {
                self.fw.running = false;
                self.say(match &r {
                    Ok(s) => s.clone(),
                    Err(e) => format!("FW update failed: {e}"),
                });
                self.fw.result = Some(r);
            }
            Ev::Logdata(r) => match r {
                Ok(s) => self.say(s),
                Err(e) => self.say(format!("logdata: {e}")),
            },
        }
    }

    fn on_session(&mut self, e: Event) {
        match e {
            Event::Liveness(l) => {
                self.conn.liveness = Some(l);
                if l == Liveness::Alive && !self.sent_on_connect {
                    self.sent_on_connect = true;
                    for c in self.settings.terminal.on_connect.clone() {
                        self.send(ports::TERMINAL, format!("{c}\r").into_bytes());
                    }
                }
            }
            Event::Log(r) => self.logs.push(&r),
            Event::DeviceInfo(v) => {
                self.device.app_hash = cbor::get_bytes(&v, "app_hash").map(<[u8]>::to_vec);
                self.device.info = Some(v);
            }
            Event::Logdata { source, targets, app_hash } => match source {
                Some(s) => {
                    self.device.logdata = Some(s.to_string());
                    self.device.targets = targets;
                }
                None => {
                    self.device.logdata = None;
                    self.say(format!(
                        "no logdata found for app_hash {}… (File → Load logdata)",
                        hex::encode(&app_hash[..app_hash.len().min(8)])
                    ));
                }
            },
            Event::Port { port, data } => {
                *self.port_counts.entry(port).or_insert(0) += 1;
                match port {
                    ports::TERMINAL => self.term.feed(&data),
                    ports::COMMANDS => {
                        let text = match cbor::from_slice(&data) {
                            Ok(v @ (Value::Map(_) | Value::Array(_))) => cbor::display(&v),
                            _ => String::from_utf8_lossy(&data).trim_end().to_string(),
                        };
                        for l in text.lines() {
                            self.cmds.responses.push_back(l.to_string());
                        }
                        while self.cmds.responses.len() > RESPONSES_MAX {
                            self.cmds.responses.pop_front();
                        }
                    }
                    ports::PLOT => {
                        self.plots.handle(&data);
                        for name in std::mem::take(&mut self.plots.created) {
                            let tab = Tab::Plot(name);
                            if let Some(d) = &mut self.dock {
                                if d.find_tab(&tab).is_none() {
                                    add_in_background(d, tab);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Console(c) => self.term.feed(&c.data),
            Event::StdinWanted => {
                self.term_view.stdin_wanted = true;
                self.say("the device is reading stdin: type a line in the Terminal");
            }
            Event::Note(n) => self.note(n),
            Event::Closed(r) => {
                self.conn.busy = matches!(r, CloseReason::HubBusy);
                self.conn.closed = Some(r.to_string());
                self.conn.liveness = None;
                self.conn.reconnecting = false;
                self.term_view.stdin_wanted = false;
                if self.fw.running {
                    self.fw.running = false;
                    self.fw.result = Some(Err("connection closed".into()));
                }
            }
        }
    }

    // ------------------------------------------------------------------------------- frame

    fn apply_look(&mut self, ctx: &egui::Context) {
        let s = self.settings.gui.scale.clamp(0.5, 3.0);
        if s != self.applied_scale {
            ctx.set_zoom_factor(s);
            self.applied_scale = s;
        }
        if self.settings.gui.theme != self.applied_theme {
            ctx.set_theme(match self.settings.gui.theme.as_str() {
                "dark" => egui::ThemePreference::Dark,
                "light" => egui::ThemePreference::Light,
                _ => egui::ThemePreference::System,
            });
            self.applied_theme = self.settings.gui.theme.clone();
        }
    }

    /// One frame (eframe and the tests both call this).
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.apply_look(&ctx);
        self.drain();
        if self.status.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(8)) {
            self.status = None;
        }
        self.shortcuts(&ctx);
        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            let mut dock = self.dock.take().unwrap_or_else(default_layout);
            DockArea::new(&mut dock)
                .style(DockStyle::from_egui(ui.style()))
                .show_inside(ui, &mut views::Viewer { app: self });
            self.dock = Some(dock);
        });
        super::connect::dialog(self, &ctx);
        super::settings_dlg::dialog(self, &ctx);
        self.about(&ctx);
        self.screenshots(&ctx);
        if self.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Keep the status line and liveness fresh without input.
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let k = |m, key| KeyboardShortcut::new(m, key);
        if ctx.input_mut(|i| i.consume_shortcut(&k(Modifiers::COMMAND, Key::K))) {
            self.open_connect();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&k(Modifiers::COMMAND, Key::L))) {
            self.load_logdata_dialog();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&k(Modifiers::COMMAND, Key::E))) {
            self.export_all();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&k(Modifiers::COMMAND, Key::Comma))) {
            self.open_settings();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&k(Modifiers::COMMAND, Key::Plus)))
            || ctx.input_mut(|i| i.consume_shortcut(&k(Modifiers::COMMAND, Key::Equals)))
        {
            self.settings.gui.scale = (self.settings.gui.scale + 0.1).min(3.0);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&k(Modifiers::COMMAND, Key::Minus))) {
            self.settings.gui.scale = (self.settings.gui.scale - 0.1).max(0.5);
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Connect…  (⌘K)").clicked() {
                    self.open_connect();
                }
                if ui.add_enabled(self.conn.uri.is_some(), egui::Button::new("Disconnect")).clicked() {
                    self.backend.send(Cmd::Disconnect);
                }
                ui.separator();
                if ui.button("Load logdata…  (⌘L)").clicked() {
                    self.load_logdata_dialog();
                }
                ui.separator();
                if ui.button("Screenshot").clicked() {
                    self.screenshot(None);
                }
                if ui.button("Export all  (⌘E)").clicked() {
                    self.export_all();
                }
                if ui.button("Set export directory…").clicked() {
                    if let Some(d) = rfd::FileDialog::new().set_directory(self.export_dir()).pick_folder() {
                        self.settings.export_dir = Some(d);
                        self.save_settings();
                    }
                }
                if ui.button("Open export directory").clicked() {
                    let d = self.export_dir();
                    let _ = std::fs::create_dir_all(&d);
                    open_path(&d);
                }
                ui.separator();
                if ui.button("Settings…  (⌘,)").clicked() {
                    self.open_settings();
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    self.quit = true;
                }
            });
            ui.menu_button("View", |ui| {
                for t in [Tab::Logs, Tab::Terminal, Tab::Commands, Tab::Device, Tab::Fw] {
                    if ui.button(t.title()).clicked() {
                        self.focus(t);
                    }
                }
                let names: Vec<String> = self.plots.plots.iter().map(|p| p.name.clone()).collect();
                if !names.is_empty() {
                    ui.menu_button("Plots", |ui| {
                        for n in names {
                            if ui.button(&n).clicked() {
                                self.focus(Tab::Plot(n));
                            }
                        }
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("UI scale");
                    if ui.button("−").clicked() {
                        self.settings.gui.scale = (self.settings.gui.scale - 0.1).max(0.5);
                    }
                    ui.label(format!("{:.0}%", self.settings.gui.scale * 100.0));
                    if ui.button("+").clicked() {
                        self.settings.gui.scale = (self.settings.gui.scale + 0.1).min(3.0);
                    }
                    if ui.button("100%").clicked() {
                        self.settings.gui.scale = 1.0;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    for (v, l) in [("system", "System"), ("dark", "Dark"), ("light", "Light")] {
                        if ui.selectable_label(self.settings.gui.theme == v, l).clicked() {
                            self.settings.gui.theme = v.into();
                        }
                    }
                });
                ui.separator();
                if ui.button("Reset layout").clicked() {
                    let mut d = default_layout();
                    for p in &self.plots.plots {
                        add_in_background(&mut d, Tab::Plot(p.name.clone()));
                    }
                    self.dock = Some(d);
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About ucLog").clicked() {
                    self.show_about = true;
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (t, c) = self.conn.status();
            let (r, p) = ui.allocate_painter(egui::vec2(10.0, 10.0), egui::Sense::hover());
            p.circle_filled(r.rect.center(), 4.5, c);
            if ui.link(RichText::new(t).color(c)).on_hover_text("Connect… (⌘K)").clicked() {
                self.open_connect();
            }
            if !self.conn.description.is_empty() {
                ui.label(RichText::new(&self.conn.description).weak());
            }
            ui.separator();
            match &self.device.logdata {
                Some(l) => {
                    // "cache /long/path/<hash>.logdata" → "cache <hash…>.logdata"; full text on hover.
                    let (kind, path) = l.split_once(' ').unwrap_or(("", l));
                    let mut name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
                    if name.len() > 40 && name.is_ascii() {
                        name = format!("{}…{}", &name[..16], &name[name.len() - 12..]);
                    }
                    ui.label(format!("logdata: {kind} {name}")).on_hover_text(l);
                }
                None => {
                    ui.label(RichText::new("no logdata").weak());
                }
            }
            ui.separator();
            ui.label(format!("logs {}", self.logs.received));
            let errs = self.logs.errors();
            if errs > 0 {
                let r = ui.label(RichText::new(format!("errors {errs}")).color(Color32::from_rgb(230, 90, 80)));
                r.on_hover_text(
                    self.logs.error_counts.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"),
                );
            }
            if self.plots.frames > 0 {
                ui.label(format!("plot frames {}", self.plots.frames));
            }
            if let (Some(p), true) = (&self.fw.progress, self.fw.running) {
                ui.label(format!("FW update: {}", crate::front::fw_ratio(p).1));
            }
            if let Some((s, _)) = &self.status {
                ui.separator();
                ui.label(RichText::new(s).italics());
            }
        });
    }

    fn about(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        egui::Window::new("About ucLog").collapsible(false).resizable(false).open(&mut self.show_about).show(
            ctx,
            |ui| {
                ui.heading(format!("ucLog {}", env!("CARGO_PKG_VERSION")));
                ui.label("Logs, terminal, commands, plots and FW update for Unit Circle devices.");
                ui.label("Keys: ⌘K connect · ⌘L load logdata · ⌘E export all · ⌘, settings · ⌘+/⌘− UI scale");
                if let Some(p) = &self.settings_path {
                    ui.label(format!("Settings: {}", p.display()));
                }
            },
        );
    }

    // ------------------------------------------------------------------------------- files

    pub fn export_dir(&self) -> PathBuf {
        self.settings.export_dir.clone().unwrap_or_else(|| self.cwd.clone())
    }

    /// Write `content` as `uclog-<stem>-<timestamp>.<ext>` in the export directory.
    pub fn export(&mut self, name: &str, content: &str) -> Option<PathBuf> {
        let dir = self.export_dir();
        let (stem, ext) = name.rsplit_once('.').unwrap_or((name, "txt"));
        let path = dir.join(format!("uclog-{stem}-{}.{ext}", crate::front::timestamp()));
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, content)) {
            Ok(()) => {
                self.say(format!("exported {}", path.display()));
                Some(path)
            }
            Err(e) => {
                self.say(format!("{}: {e}", path.display()));
                None
            }
        }
    }

    /// Logs, terminal and every plot, with one timestamp.
    pub fn export_all(&mut self) {
        let dir = self.export_dir();
        let ts = crate::front::timestamp();
        let mut files: Vec<(String, String)> = vec![
            (format!("uclog-{ts}-logs.csv"), self.logs.csv()),
            (format!("uclog-{ts}-terminal.txt"), self.term.text()),
        ];
        for p in &self.plots.plots {
            if let Some(c) = self.plots.csv(&p.name) {
                files.push((format!("uclog-{ts}-plot-{}.csv", safe_name(&p.name)), c));
            }
        }
        let r = std::fs::create_dir_all(&dir).and_then(|_| {
            for (n, c) in &files {
                std::fs::write(dir.join(n), c)?;
            }
            Ok(())
        });
        match r {
            Ok(()) => self.say(format!("exported {} files to {}", files.len(), dir.display())),
            Err(e) => self.say(format!("{}: {e}", dir.display())),
        }
    }

    pub fn load_logdata_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Load logdata")
            .add_filter("logdata / ELF", &["logdata", "elf", "cbor"])
            .add_filter("All files", &["*"])
            .set_directory(&self.cwd)
            .pick_file()
        {
            self.backend.send(Cmd::LoadLogdata(p, self.device.app_hash.clone()));
        }
    }

    pub fn open_settings(&mut self) {
        let s = self.settings.clone();
        let json = |b: &Vec<settings::Button>| serde_json::to_string_pretty(b).unwrap_or_default();
        self.settings_dlg = Some(SettingsDialog {
            open: true,
            usb: s.usb_filters.join(", "),
            on_connect: s.terminal.on_connect.join("\n"),
            term_buttons: json(&s.terminal.buttons),
            cmd_buttons: json(&s.commands.buttons),
            token: String::new(),
            token_set: settings::keyring_token().is_some(),
            error: None,
            s,
        });
    }

    /// Ask for a screenshot; it is saved when it arrives (`screenshots`).
    pub fn screenshot(&mut self, path: Option<PathBuf>) {
        let path = path
            .unwrap_or_else(|| self.export_dir().join(format!("uclog-screenshot-{}.png", crate::front::timestamp())));
        self.script.screenshot = Some(path);
        self.script.requested = false;
        self.script.show_done = false;
        self.script.at = Some(Instant::now());
    }

    fn screenshots(&mut self, ctx: &egui::Context) {
        let Some(path) = self.script.screenshot.clone() else { return };
        if !self.script.show_done && self.script.at.is_some_and(|t| Instant::now() >= t) {
            for s in std::mem::take(&mut self.script.show) {
                let tab = match s.as_str() {
                    "Logs" => Tab::Logs,
                    "Terminal" => Tab::Terminal,
                    "Commands" => Tab::Commands,
                    "Device" => Tab::Device,
                    "FW" => Tab::Fw,
                    _ => Tab::Plot(s),
                };
                self.focus(tab);
            }
            // Let the focused views draw once before the capture.
            ctx.request_repaint();
            self.script.at = Some(Instant::now() + Duration::from_millis(300));
            self.script.show_done = true;
            return;
        }
        if self.script.show_done && !self.script.requested && self.script.at.is_some_and(|t| Instant::now() >= t) {
            self.script.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            self.script.screenshot = None;
            let [w, h] = img.size;
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            let r =
                std::fs::create_dir_all(path.parent().unwrap_or(&self.cwd)).map_err(|e| e.to_string()).and_then(|_| {
                    image::save_buffer(&path, &rgba, w as u32, h as u32, image::ColorType::Rgba8)
                        .map_err(|e| e.to_string())
                });
            match r {
                Ok(()) => self.say(format!("screenshot {}", path.display())),
                Err(e) => self.say(format!("{}: {e}", path.display())),
            }
            if self.script.exit {
                // Exit directly: closing a window that was never active right after it opened
                // can make AppKit throw (winit Touch Bar observer) on macOS.
                std::process::exit(0);
            }
        } else if self.script.requested {
            ctx.request_repaint();
        }
    }
}

pub fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect()
}

/// Open a file or directory with the OS (Finder, Explorer, xdg-open).
pub fn open_path(p: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(p).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(p).spawn();
    let _ = r;
}
