//! TUI state and input handling. Pure: it turns session events and key presses into state
//! changes and [`Action`]s, which the runner (`mod.rs`) carries out.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use regex::Regex;
use uclog_link::{Device, LinkSpec};
use uclog_proto::cbor::{self, Value};
use uclog_proto::ports;
use uclog_session::{Event, FwProgress, Liveness};

use super::lineedit::{Edit, LineEdit};
use super::logs::{Filter, LogStore, LEVELS};
use super::picker::{Pick, Picker};
pub use crate::front::{button_args, button_payload, strip_ansi, Answer};
use crate::settings::Settings;

pub const TERM_LINES_MAX: usize = 5000;
pub const RESPONSES_MAX: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Logs,
    Terminal,
    Commands,
    Device,
    Fw,
}

pub const TABS: [(Tab, &str); 5] = [
    (Tab::Logs, "1 Logs"),
    (Tab::Terminal, "2 Terminal"),
    (Tab::Commands, "3 Commands"),
    (Tab::Device, "4 Device"),
    (Tab::Fw, "5 FW Update"),
];

/// What the runner must do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    Connect(LinkSpec),
    Disconnect,
    Send(u8, Vec<u8>),
    Stdin(Vec<u8>),
    LoadLogdata(PathBuf),
    FwStart(PathBuf),
    FwCancel,
    RefreshDevices,
    BleScan,
    SaveSettings { token: Option<Option<String>> },
    SetFilesDir(Option<PathBuf>),
    Copy(String),
    Export { name: String, content: String },
}

/// Results of background work, fed back by the runner.
#[derive(Debug, Clone)]
pub enum Bg {
    Connecting(String),
    Connected { uri: String, description: String },
    ConnectFailed(String),
    Devices(Vec<Device>),
    BleDevices(Result<Vec<Device>, String>),
    Fw(FwProgress),
    FwDone(Result<String, String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickFor {
    Logdata,
    FwImage,
}

/// A one-line question.
pub struct Prompt {
    pub title: String,
    pub input: LineEdit,
    pub then: Then,
    pub secret: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Then {
    SourceFilter,
    LineFilter,
    Search,
    AddAddress,
    /// A `$arg` of a command button: answers so far as (key, placeholder, answer), and the
    /// (key, placeholder) pairs still to ask.
    CommandArg {
        button: usize,
        answers: Vec<Answer>,
        remaining: Vec<(String, String)>,
    },
    Setting(usize),
    ConfirmFw(PathBuf),
    RemoveToken,
}

pub enum Modal {
    None,
    Help,
    Connect,
    Settings,
    Picker(Box<Picker>, PickFor),
    Prompt(Prompt),
}

#[derive(Debug, Clone, Default)]
pub struct ConnState {
    pub uri: Option<String>,
    pub description: String,
    pub liveness: Option<Liveness>,
    pub connecting: Option<String>,
    pub closed: Option<String>,
}

#[derive(Default)]
pub struct TermState {
    pub lines: VecDeque<String>,
    pub partial: String,
    pub input: LineEdit,
    pub raw: bool,
    /// Lines scrolled up from the bottom.
    pub scroll: usize,
    pub stdin_wanted: bool,
}

impl TermState {
    pub fn append(&mut self, data: &[u8]) {
        let s = String::from_utf8_lossy(data);
        for c in s.chars() {
            match c {
                '\n' => {
                    let l = std::mem::take(&mut self.partial);
                    self.lines.push_back(l);
                    if self.lines.len() > TERM_LINES_MAX {
                        self.lines.pop_front();
                    }
                }
                '\r' => {}
                '\u{8}' => {
                    self.partial.pop();
                }
                c => self.partial.push(c),
            }
        }
    }

    pub fn text(&self) -> String {
        let mut s: String = self.lines.iter().map(|l| format!("{l}\n")).collect();
        s.push_str(&self.partial);
        s
    }
}

#[derive(Default)]
pub struct CmdState {
    pub selected: usize,
    pub input: LineEdit,
    pub responses: VecDeque<String>,
    /// Focus: false = buttons, true = the text input.
    pub typing: bool,
}

#[derive(Default)]
pub struct FwState {
    pub file: Option<PathBuf>,
    pub progress: Option<FwProgress>,
    pub running: bool,
    pub result: Option<Result<String, String>>,
    pub started: Option<Instant>,
}

#[derive(Default)]
pub struct DeviceState {
    pub info: Option<Value>,
    pub logdata: Option<String>,
    pub targets: Vec<u8>,
    pub notes: VecDeque<String>,
}

pub struct App {
    pub settings: Settings,
    pub tab: Tab,
    pub modal: Modal,
    pub conn: ConnState,
    pub logs: LogStore,
    /// Selected visible row; `None` = follow the newest.
    pub log_sel: Option<usize>,
    pub paused: bool,
    /// In the Logs tab: keys go to the terminal input below the logs.
    pub logs_term_focus: bool,
    pub term: TermState,
    pub cmds: CmdState,
    pub device: DeviceState,
    pub fw: FwState,
    pub port_counts: BTreeMap<u8, u64>,
    pub plot_frames: u64,
    pub devices: Vec<Device>,
    pub devices_sel: usize,
    pub scanning: bool,
    pub settings_sel: usize,
    /// Token as edited in Settings: `None` = unchanged.
    pub token_edit: Option<Option<String>>,
    pub token_set: bool,
    /// The session's token is the keyring's (so removing it there helps).
    pub token_from_keyring: bool,
    /// Asked about removing a refused token in this run.
    pub token_asked: bool,
    pub status: Option<(String, Instant)>,
    pub cwd: PathBuf,
    sent_on_connect: bool,
}

/// (key, placeholder, answer) for a command button's `$arg`.
/// Bytes for one key in character mode (as `uclog term --raw`).
pub fn key_bytes(k: KeyEvent) -> Option<Vec<u8>> {
    Some(match k.code {
        KeyCode::Char(c) if k.modifiers.contains(KeyModifiers::CONTROL) => {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() {
                vec![c as u8 - b'a' + 1]
            } else {
                return None;
            }
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Backspace => vec![0x7F],
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::Esc => vec![0x1B],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => return None,
    })
}

pub const SETTINGS_FIELDS: [&str; 7] = [
    "Logdata domain",
    "Logdata token",
    "Logdata URL",
    "USB filters (VID:PID, comma separated)",
    "Files directory (semihost)",
    "Export directory",
    "Max log lines",
];

impl App {
    pub fn new(settings: Settings, cwd: PathBuf) -> Self {
        let history = settings.terminal.history.clone();
        App {
            logs: LogStore::new(settings.max_log_lines),
            settings,
            tab: Tab::Logs,
            modal: Modal::None,
            conn: ConnState::default(),
            log_sel: None,
            paused: false,
            logs_term_focus: false,
            term: TermState { input: LineEdit::with_history(history), ..Default::default() },
            cmds: CmdState::default(),
            device: DeviceState::default(),
            fw: FwState::default(),
            port_counts: BTreeMap::new(),
            plot_frames: 0,
            devices: vec![],
            devices_sel: 0,
            scanning: false,
            settings_sel: 0,
            token_edit: None,
            token_set: false,
            token_from_keyring: true,
            token_asked: false,
            status: None,
            cwd,
            sent_on_connect: false,
        }
    }

    pub fn say(&mut self, s: impl Into<String>) {
        self.status = Some((s.into(), Instant::now()));
    }

    /// The service refused the token but read without it worked: offer to remove the saved token
    /// (once per run), unless it came from the command line or the environment.
    fn token_refused(&mut self) {
        if !(self.token_from_keyring && self.token_set) {
            self.say("the logdata token was refused, but the account allows reading without one: drop --logdata-token or unset UCLOG_LOGDATA_TOKEN");
        } else if !self.token_asked && matches!(self.modal, Modal::None) {
            self.token_asked = true;
            self.prompt(
                "Token refused, but reading without one works. Remove the saved token? (y/n)",
                Then::RemoveToken,
                false,
            );
        }
    }

    fn note(&mut self, s: String) {
        self.device.notes.push_back(s.clone());
        if self.device.notes.len() > 50 {
            self.device.notes.pop_front();
        }
        self.say(s);
    }

    // -----------------------------------------------------------------------------------------
    // Session and background events

    pub fn on_session(&mut self, e: Event) -> Vec<Action> {
        let mut out = vec![];
        match e {
            Event::Liveness(l) => {
                self.conn.liveness = Some(l);
                if l == Liveness::Alive && !self.sent_on_connect {
                    self.sent_on_connect = true;
                    for c in self.settings.terminal.on_connect.clone() {
                        out.push(Action::Send(ports::TERMINAL, format!("{c}\r").into_bytes()));
                    }
                }
            }
            Event::Log(r) => {
                if !self.paused {
                    self.logs.push(&r);
                } else {
                    // Paused: keep storing, but do not move the view.
                    self.logs.push(&r);
                    if self.log_sel.is_none() {
                        self.log_sel = Some(self.logs.visible_len().saturating_sub(1));
                    }
                }
            }
            Event::DeviceInfo(v) => self.device.info = Some(v),
            Event::Logdata { source, targets, app_hash } => match source {
                Some(s) => {
                    self.device.logdata = Some(s.to_string());
                    self.device.targets = targets;
                }
                None => {
                    self.device.logdata = None;
                    self.note(format!(
                        "no logdata found for app_hash {}…",
                        hex::encode(&app_hash[..app_hash.len().min(8)])
                    ));
                }
            },
            Event::Port { port, data } => {
                *self.port_counts.entry(port).or_insert(0) += 1;
                match port {
                    ports::TERMINAL => self.term.append(&data),
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
                    ports::PLOT => self.plot_frames += 1,
                    _ => {}
                }
            }
            Event::Console(c) => self.term.append(&c.data),
            Event::StdinWanted => {
                self.term.stdin_wanted = true;
                self.say("the device is reading stdin: type a line in the terminal");
            }
            Event::Note(n) => self.note(n),
            Event::TokenRefused => self.token_refused(),
            Event::Closed(r) => {
                self.conn.closed = Some(r.to_string());
                self.conn.liveness = None;
                self.term.stdin_wanted = false;
                if self.fw.running {
                    self.fw.running = false;
                    self.fw.result = Some(Err("connection closed".into()));
                }
            }
        }
        out
    }

    pub fn on_bg(&mut self, b: Bg) {
        match b {
            Bg::Connecting(u) => {
                self.conn.connecting = Some(u);
                self.conn.closed = None;
            }
            Bg::Connected { uri, description } => {
                self.conn =
                    ConnState { uri: Some(uri), description, liveness: Some(Liveness::Waiting), ..Default::default() };
                self.sent_on_connect = false;
                self.device = DeviceState::default();
            }
            Bg::ConnectFailed(e) => {
                self.conn.connecting = None;
                self.conn.closed = Some(e.clone());
                self.say(e);
            }
            Bg::Devices(d) => {
                // Keep BLE results from an earlier scan.
                let ble: Vec<Device> =
                    self.devices.iter().filter(|x| x.kind == uclog_link::Kind::Ble).cloned().collect();
                self.devices = d;
                self.devices.extend(ble);
                self.devices_sel = self.devices_sel.min(self.devices.len().saturating_sub(1));
            }
            Bg::BleDevices(r) => {
                self.scanning = false;
                match r {
                    Ok(v) => {
                        self.devices.retain(|x| x.kind != uclog_link::Kind::Ble);
                        let n = v.len();
                        self.devices.extend(v);
                        self.say(format!("BLE scan: {n} device(s)"));
                    }
                    Err(e) => self.say(format!("BLE scan: {e}")),
                }
            }
            Bg::Fw(p) => self.fw.progress = Some(p),
            Bg::FwDone(r) => {
                self.fw.running = false;
                self.say(match &r {
                    Ok(s) => s.clone(),
                    Err(e) => format!("FW update failed: {e}"),
                });
                self.fw.result = Some(r);
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Keys

    pub fn on_key(&mut self, k: KeyEvent) -> Vec<Action> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('q')) && !self.raw_terminal_focused() {
            return vec![Action::Quit];
        }
        if ctrl && k.code == KeyCode::Char('q') {
            return vec![Action::Quit];
        }
        match std::mem::replace(&mut self.modal, Modal::None) {
            Modal::None => {}
            Modal::Help => {
                if !matches!(k.code, KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('q') | KeyCode::Enter) {
                    self.modal = Modal::Help;
                }
                return vec![];
            }
            Modal::Connect => return self.connect_key(k),
            Modal::Settings => return self.settings_key(k),
            Modal::Picker(mut p, why) => {
                return match p.key(k) {
                    Pick::None => {
                        self.modal = Modal::Picker(p, why);
                        vec![]
                    }
                    Pick::Cancel => vec![],
                    Pick::Chosen(path) => match why {
                        PickFor::Logdata => vec![Action::LoadLogdata(path)],
                        PickFor::FwImage => {
                            self.fw.file = Some(path.clone());
                            self.prompt(
                                &format!("Update the firmware with {}? (y/n)", path.display()),
                                Then::ConfirmFw(path),
                                false,
                            );
                            vec![]
                        }
                    },
                };
            }
            Modal::Prompt(mut p) => {
                if k.code == KeyCode::Esc {
                    return vec![];
                }
                return match p.input.key(k) {
                    Edit::Submit(s) => self.prompt_done(p.then, s),
                    _ => {
                        self.modal = Modal::Prompt(p);
                        vec![]
                    }
                };
            }
        }

        // Global keys.
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Char(c @ '1'..='5') if alt => {
                self.tab = TABS[(c as u8 - b'1') as usize].0;
                return vec![];
            }
            KeyCode::F(1) => {
                self.modal = Modal::Help;
                return vec![];
            }
            KeyCode::F(2) => {
                self.modal = Modal::Connect;
                return vec![Action::RefreshDevices];
            }
            KeyCode::F(3) => {
                self.open_logdata_picker();
                return vec![];
            }
            KeyCode::F(5) => {
                self.tab = Tab::Fw;
                return vec![];
            }
            KeyCode::F(n @ 6..=12) => {
                let i = (n - 6) as usize;
                return match self.settings.terminal.buttons.get(i).cloned() {
                    Some(b) => vec![Action::Send(ports::TERMINAL, button_payload(&b, &[]))],
                    None => vec![],
                };
            }
            KeyCode::Char('s') if ctrl => {
                self.modal = Modal::Settings;
                return vec![];
            }
            KeyCode::Char('e') if ctrl => return self.export(),
            KeyCode::Char('l') if ctrl => {
                match self.tab {
                    Tab::Logs if !self.logs_term_focus => {
                        self.logs.clear();
                        self.log_sel = None;
                    }
                    Tab::Commands => self.cmds.responses.clear(),
                    _ => {
                        self.term.lines.clear();
                        self.term.partial.clear();
                    }
                }
                return vec![];
            }
            KeyCode::Char('t') if ctrl => {
                self.term.raw = !self.term.raw;
                self.say(if self.term.raw {
                    "terminal: character mode (Ctrl+T for line mode)"
                } else {
                    "terminal: line mode"
                });
                return vec![];
            }
            _ => {}
        }
        if self.input_focused() {
            return self.terminal_key(k);
        }
        if self.tab == Tab::Commands && self.cmds.typing {
            return self.commands_key(k);
        }
        // Tab switching and quit (only when not typing).
        match k.code {
            KeyCode::Char(c @ '1'..='5') => {
                self.tab = TABS[(c as u8 - b'1') as usize].0;
                return vec![];
            }
            KeyCode::Char('q') => return vec![Action::Quit],
            _ => {}
        }
        match self.tab {
            Tab::Logs => self.logs_key(k),
            Tab::Terminal => self.terminal_key(k),
            Tab::Commands => self.commands_key(k),
            Tab::Device => self.device_key(k),
            Tab::Fw => self.fw_key(k),
        }
    }

    fn input_focused(&self) -> bool {
        self.tab == Tab::Terminal || (self.tab == Tab::Logs && self.logs_term_focus)
    }

    fn raw_terminal_focused(&self) -> bool {
        self.term.raw && matches!(self.modal, Modal::None) && self.input_focused()
    }

    pub fn prompt(&mut self, title: &str, then: Then, secret: bool) {
        self.modal = Modal::Prompt(Prompt { title: title.into(), input: LineEdit::default(), then, secret });
    }

    fn prompt_with(&mut self, title: &str, then: Then, initial: &str) {
        let mut input = LineEdit::default();
        input.set(initial);
        self.modal = Modal::Prompt(Prompt { title: title.into(), input, then, secret: false });
    }

    fn prompt_done(&mut self, then: Then, s: String) -> Vec<Action> {
        match then {
            Then::SourceFilter => {
                let mut f = self.logs.filter.clone();
                f.source = s.trim().into();
                self.set_filter(f);
            }
            Then::LineFilter => {
                let mut f = self.logs.filter.clone();
                f.line = s.trim().into();
                self.set_filter(f);
            }
            Then::Search => {
                let mut f = self.logs.filter.clone();
                if s.is_empty() {
                    f.search = None;
                } else {
                    match Regex::new(&s) {
                        Ok(re) => f.search = Some(re),
                        Err(e) => {
                            self.say(format!("bad regex: {e}"));
                            return vec![];
                        }
                    }
                }
                self.set_filter(f);
            }
            Then::AddAddress => {
                let s = s.trim();
                let s = if s.contains("://")
                    || s.contains(':') && !s.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ':')
                {
                    s.to_string()
                } else {
                    format!("tcp://{s}")
                };
                match s.parse::<LinkSpec>() {
                    Ok(spec) => return vec![Action::Connect(spec)],
                    Err(e) => self.say(e.to_string()),
                }
            }
            Then::CommandArg { button, mut answers, mut remaining } => {
                let (key, arg) = remaining.remove(0);
                answers.push((key, arg, s));
                return self.next_command_arg(button, answers, remaining);
            }
            Then::Setting(i) => return self.set_setting(i, s),
            Then::RemoveToken => {
                if s.trim().eq_ignore_ascii_case("y") || s.trim().eq_ignore_ascii_case("yes") {
                    self.token_set = false;
                    return vec![Action::SaveSettings { token: Some(None) }];
                }
            }
            Then::ConfirmFw(p) => {
                if s.trim().eq_ignore_ascii_case("y") || s.trim().eq_ignore_ascii_case("yes") {
                    self.fw.running = true;
                    self.fw.progress = None;
                    self.fw.result = None;
                    self.fw.started = Some(Instant::now());
                    self.tab = Tab::Fw;
                    return vec![Action::FwStart(p)];
                }
            }
        }
        vec![]
    }

    fn set_filter(&mut self, f: Filter) {
        self.logs.set_filter(f);
        self.log_sel = None;
    }

    // Logs ------------------------------------------------------------------------------------

    fn logs_key(&mut self, k: KeyEvent) -> Vec<Action> {
        let n = self.logs.visible_len();
        let cur = self.log_sel.unwrap_or(n.saturating_sub(1));
        let mut f = self.logs.filter.clone();
        match k.code {
            KeyCode::Tab => self.logs_term_focus = true,
            KeyCode::Up => self.log_sel = Some(cur.saturating_sub(1)),
            KeyCode::Down => self.log_sel = if cur + 1 >= n { None } else { Some(cur + 1) },
            KeyCode::PageUp => self.log_sel = Some(cur.saturating_sub(20)),
            KeyCode::PageDown => self.log_sel = if cur + 20 >= n { None } else { Some(cur + 20) },
            KeyCode::Home => self.log_sel = Some(0),
            KeyCode::End | KeyCode::Esc => self.log_sel = None,
            KeyCode::Char('+') | KeyCode::Char('=') => {
                f.min_level = (f.min_level + 1).min(LEVELS.len() as u8 - 1);
                self.set_filter(f);
            }
            KeyCode::Char('-') => {
                f.min_level = f.min_level.saturating_sub(1);
                self.set_filter(f);
            }
            KeyCode::Char('/') => self.prompt_with(
                "Search (regex, empty = none)",
                Then::Search,
                &f.search.map(|r| r.to_string()).unwrap_or_default(),
            ),
            KeyCode::Char('f') => self.prompt_with("Source filter (file name contains)", Then::SourceFilter, &f.source),
            KeyCode::Char('n') => self.prompt_with("Line filter (line number)", Then::LineFilter, &f.line),
            KeyCode::Char('s') => {
                if let Some(r) = self.logs.visible(cur) {
                    f.source = r.file.clone();
                    self.set_filter(f);
                }
            }
            KeyCode::Char('x') => self.set_filter(Filter::default()),
            KeyCode::Char('p') => {
                self.paused = !self.paused;
                if self.paused && self.log_sel.is_none() {
                    self.log_sel = Some(n.saturating_sub(1));
                } else if !self.paused {
                    self.log_sel = None;
                }
            }
            KeyCode::Char('d') => {
                self.logs.discard_filtered = !self.logs.discard_filtered;
                self.say(if self.logs.discard_filtered {
                    "discarding filtered lines"
                } else {
                    "keeping filtered lines"
                });
            }
            KeyCode::Char('c') | KeyCode::Char('y') => {
                if let Some(r) = self.logs.visible(cur) {
                    return vec![Action::Copy(r.plain())];
                }
            }
            _ => {}
        }
        vec![]
    }

    // Terminal --------------------------------------------------------------------------------

    fn terminal_key(&mut self, k: KeyEvent) -> Vec<Action> {
        match k.code {
            KeyCode::PageUp => {
                self.term.scroll = (self.term.scroll + 10).min(self.term.lines.len());
                return vec![];
            }
            KeyCode::PageDown => {
                self.term.scroll = self.term.scroll.saturating_sub(10);
                return vec![];
            }
            KeyCode::Tab if self.tab == Tab::Logs => {
                self.logs_term_focus = false;
                return vec![];
            }
            KeyCode::Esc if self.tab == Tab::Logs && !self.term.raw => {
                self.logs_term_focus = false;
                return vec![];
            }
            KeyCode::Esc if self.tab == Tab::Terminal && !self.term.raw => {
                self.tab = Tab::Logs;
                return vec![];
            }
            _ => {}
        }
        if self.term.raw && !self.term.stdin_wanted {
            self.term.scroll = 0;
            return key_bytes(k).map(|b| vec![Action::Send(ports::TERMINAL, b)]).unwrap_or_default();
        }
        match self.term.input.key(k) {
            Edit::Submit(line) => {
                self.term.scroll = 0;
                if self.term.stdin_wanted {
                    self.term.stdin_wanted = false;
                    self.term.append(format!("{line}\n").as_bytes());
                    return vec![Action::Stdin(format!("{line}\n").into_bytes())];
                }
                self.term.input.push_history(&line);
                self.settings.push_history(&line);
                vec![Action::Send(ports::TERMINAL, format!("{line}\r").into_bytes())]
            }
            _ => vec![],
        }
    }

    // Commands --------------------------------------------------------------------------------

    fn commands_key(&mut self, k: KeyEvent) -> Vec<Action> {
        if self.cmds.typing {
            if k.code == KeyCode::Esc || k.code == KeyCode::Tab {
                self.cmds.typing = false;
                return vec![];
            }
            return match self.cmds.input.key(k) {
                Edit::Submit(line) if !line.is_empty() => {
                    self.cmds.input.push_history(&line);
                    self.cmds.responses.push_back(format!("> {line}"));
                    vec![Action::Send(ports::COMMANDS, format!("{line}\r").into_bytes())]
                }
                _ => vec![],
            };
        }
        let n = self.settings.commands.buttons.len();
        match k.code {
            KeyCode::Tab | KeyCode::Char('i') => self.cmds.typing = true,
            KeyCode::Up => self.cmds.selected = self.cmds.selected.saturating_sub(1),
            KeyCode::Down => self.cmds.selected = (self.cmds.selected + 1).min(n.saturating_sub(1)),
            KeyCode::Enter if n > 0 => {
                let i = self.cmds.selected.min(n - 1);
                let remaining = button_args(&self.settings.commands.buttons[i]);
                return self.next_command_arg(i, vec![], remaining);
            }
            _ => {}
        }
        vec![]
    }

    fn next_command_arg(
        &mut self,
        button: usize,
        answers: Vec<Answer>,
        remaining: Vec<(String, String)>,
    ) -> Vec<Action> {
        let b = self.settings.commands.buttons[button].clone();
        if let Some((key, arg)) = remaining.first() {
            let title = format!("{}: {key} = {arg}", b.label);
            self.prompt(&title, Then::CommandArg { button, answers, remaining }, false);
            return vec![];
        }
        self.cmds.responses.push_back(format!("> {}", b.label));
        vec![Action::Send(ports::COMMANDS, button_payload(&b, &answers))]
    }

    // Device / FW -----------------------------------------------------------------------------

    fn open_logdata_picker(&mut self) {
        let sug = crate::front::signed_images(&self.cwd, 4)
            .into_iter()
            .filter_map(|p| p.parent().map(|d| d.join("zephyr.elf")))
            .filter(|p| p.exists())
            .collect();
        self.modal = Modal::Picker(
            Box::new(Picker::new("Load logdata (.logdata, .elf)", &["logdata", "elf", "cbor"], self.cwd.clone(), sug)),
            PickFor::Logdata,
        );
    }

    fn device_key(&mut self, k: KeyEvent) -> Vec<Action> {
        match k.code {
            KeyCode::Char('l') | KeyCode::Enter => self.open_logdata_picker(),
            KeyCode::Char('c') => {
                if let Some(h) = self.device.info.as_ref().and_then(|i| cbor::get_bytes(i, "app_hash")) {
                    return vec![Action::Copy(hex::encode(h))];
                }
            }
            _ => {}
        }
        vec![]
    }

    fn fw_key(&mut self, k: KeyEvent) -> Vec<Action> {
        match k.code {
            KeyCode::Esc | KeyCode::Char('c') if self.fw.running => {
                self.fw.running = false;
                self.fw.result = Some(Err("cancelled".into()));
                return vec![Action::FwCancel];
            }
            KeyCode::Enter | KeyCode::Char('o') if !self.fw.running => {
                let sug = crate::front::signed_images(&self.cwd, 4);
                self.modal = Modal::Picker(
                    Box::new(Picker::new("FW image (.bin)", &["bin"], self.cwd.clone(), sug)),
                    PickFor::FwImage,
                );
            }
            _ => {}
        }
        vec![]
    }

    // Connect ---------------------------------------------------------------------------------

    fn connect_key(&mut self, k: KeyEvent) -> Vec<Action> {
        let n = self.devices.len();
        match k.code {
            KeyCode::Esc => return vec![],
            KeyCode::Up => self.devices_sel = self.devices_sel.saturating_sub(1),
            KeyCode::Down => self.devices_sel = (self.devices_sel + 1).min(n.saturating_sub(1)),
            KeyCode::Char('r') => {
                self.modal = Modal::Connect;
                return vec![Action::RefreshDevices];
            }
            KeyCode::Char('s') => {
                self.scanning = true;
                self.modal = Modal::Connect;
                return vec![Action::BleScan];
            }
            KeyCode::Char('a') => {
                self.prompt(
                    "Address: tcp://host[:9000], ws://…, udp://host:port, serial:…, loopback:",
                    Then::AddAddress,
                    false,
                );
                return vec![];
            }
            KeyCode::Char('d') => return vec![Action::Disconnect],
            KeyCode::Enter => {
                if let Some(d) = self.devices.get(self.devices_sel) {
                    match d.uri.parse::<LinkSpec>() {
                        Ok(s) => return vec![Action::Connect(s)],
                        Err(e) => self.say(e.to_string()),
                    }
                }
            }
            _ => {}
        }
        self.modal = Modal::Connect;
        vec![]
    }

    // Settings --------------------------------------------------------------------------------

    pub fn setting_value(&self, i: usize) -> String {
        let s = &self.settings;
        match i {
            0 => s.logdata.domain.clone().unwrap_or_default(),
            1 => match &self.token_edit {
                Some(Some(_)) => "•••••• (changed)".into(),
                Some(None) => "(removed)".into(),
                None if self.token_set => "•••••• (keyring)".into(),
                None => String::new(),
            },
            2 => s.logdata.url.clone().unwrap_or_default(),
            3 => s.usb_filters.join(", "),
            4 => s.files_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            5 => s.export_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            6 => s.max_log_lines.to_string(),
            _ => String::new(),
        }
    }

    fn settings_key(&mut self, k: KeyEvent) -> Vec<Action> {
        match k.code {
            KeyCode::Esc => return vec![],
            KeyCode::Up => self.settings_sel = self.settings_sel.saturating_sub(1),
            KeyCode::Down => self.settings_sel = (self.settings_sel + 1).min(SETTINGS_FIELDS.len() - 1),
            KeyCode::Enter => {
                let i = self.settings_sel;
                let cur = if i == 1 { String::new() } else { self.setting_value(i) };
                let mut input = LineEdit::default();
                input.set(&cur);
                self.modal = Modal::Prompt(Prompt {
                    title: format!("{} (empty = default/none)", SETTINGS_FIELDS[i]),
                    input,
                    then: Then::Setting(i),
                    secret: i == 1,
                });
                return vec![];
            }
            KeyCode::Char('s') => {
                let token = self.token_edit.take();
                if let Some(t) = &token {
                    self.token_set = t.is_some();
                }
                return vec![Action::SaveSettings { token }];
            }
            _ => {}
        }
        self.modal = Modal::Settings;
        vec![]
    }

    fn set_setting(&mut self, i: usize, v: String) -> Vec<Action> {
        let v = v.trim().to_string();
        let opt = |v: &str| (!v.is_empty()).then(|| v.to_string());
        let mut out = vec![];
        match i {
            0 => self.settings.logdata.domain = opt(&v),
            1 => self.token_edit = Some(opt(&v)),
            2 => self.settings.logdata.url = opt(&v),
            3 => {
                self.settings.usb_filters =
                    v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
            }
            4 => {
                self.settings.files_dir = opt(&v).map(PathBuf::from);
                out.push(Action::SetFilesDir(self.settings.files_dir.clone()));
            }
            5 => self.settings.export_dir = opt(&v).map(PathBuf::from),
            6 => match v.parse::<usize>() {
                Ok(n) if n >= 100 => {
                    self.settings.max_log_lines = n;
                    self.logs.set_max(n);
                }
                _ => self.say("max log lines: a number ≥ 100"),
            },
            _ => {}
        }
        self.modal = Modal::Settings;
        out
    }

    // Export ----------------------------------------------------------------------------------

    fn export(&mut self) -> Vec<Action> {
        match self.tab {
            Tab::Logs if !self.logs_term_focus => {
                vec![Action::Export { name: "logs.csv".into(), content: self.logs.csv() }]
            }
            Tab::Commands => {
                let t: String = self.cmds.responses.iter().map(|l| format!("{l}\n")).collect();
                vec![Action::Export { name: "commands.txt".into(), content: t }]
            }
            _ => vec![Action::Export { name: "terminal.txt".into(), content: strip_ansi(&self.term.text()) }],
        }
    }
}
