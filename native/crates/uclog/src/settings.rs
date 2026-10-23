//! Settings shared by the front ends (TUI and GUI): `settings.json` in the platform config
//! directory, the logdata token in the OS keyring, and a one-time import of the legacy
//! `~/ucLogger.json` (buttons, history, max lines, files directory, source colours).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "ca.unitcircle.uclog";
const KEYRING_TOKEN: &str = "logdata-token";

/// A configured button: `cmd` is a string (sent as a text line) or a JSON object (sent as a CBOR
/// map; string values may contain `$name` placeholders that are asked for when pressed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Button {
    pub label: String,
    pub cmd: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogdataSettings {
    pub domain: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    pub buttons: Vec<Button>,
    /// Most recent last; at most [`HISTORY_MAX`].
    pub history: Vec<String>,
    /// Lines sent on port 61 after connecting.
    pub on_connect: Vec<String>,
}

pub const HISTORY_MAX: usize = 50;

impl Default for TerminalSettings {
    fn default() -> Self {
        TerminalSettings {
            buttons: vec![Button { label: "Help".into(), cmd: "help".into() }],
            history: vec![],
            on_connect: vec![],
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandSettings {
    pub buttons: Vec<Button>,
}

/// GUI-only settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiSettings {
    /// UI zoom factor.
    pub scale: f32,
    /// `system`, `dark` or `light`.
    pub theme: String,
    /// Log row background per source (file name without directory and extension, as the legacy
    /// GUI keyed it). Sources without an entry get a colour derived from their name.
    pub log_colors: BTreeMap<String, [u8; 3]>,
}

impl Default for GuiSettings {
    fn default() -> Self {
        GuiSettings { scale: 1.0, theme: "system".into(), log_colors: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub logdata: LogdataSettings,
    /// `VID:PID` entries; empty = the defaults.
    pub usb_filters: Vec<String>,
    /// Semihost sandbox root.
    pub files_dir: Option<PathBuf>,
    /// CSV exports go here (default: the current directory).
    pub export_dir: Option<PathBuf>,
    pub max_log_lines: usize,
    pub terminal: TerminalSettings,
    pub commands: CommandSettings,
    pub gui: GuiSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            logdata: LogdataSettings::default(),
            usb_filters: vec![],
            files_dir: None,
            export_dir: None,
            max_log_lines: 10_000,
            terminal: TerminalSettings::default(),
            commands: CommandSettings::default(),
            gui: GuiSettings::default(),
        }
    }
}

pub fn default_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("ca", "unitcircle", "uclog").map(|d| d.config_dir().join("settings.json"))
}

fn legacy_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().join("ucLogger.json"))
}

impl Settings {
    /// Load from `path` (or the default). When there is no settings file yet, import the legacy
    /// `~/ucLogger.json` if present. Returns the settings, the path to save to, and a note.
    pub fn load(path: Option<&Path>) -> (Settings, Option<PathBuf>, Option<String>) {
        let path = path.map(Path::to_path_buf).or_else(default_path);
        let Some(p) = &path else { return (Settings::default(), None, None) };
        match std::fs::read_to_string(p) {
            Ok(t) => match serde_json::from_str(&t) {
                Ok(s) => (s, path, None),
                Err(e) => (Settings::default(), path.clone(), Some(format!("{}: {e} (using defaults)", p.display()))),
            },
            Err(_) => match legacy_path().filter(|l| l.exists()) {
                Some(l) => match Settings::import_legacy(&l) {
                    Ok(s) => (s, path, Some(format!("imported settings from {}", l.display()))),
                    Err(e) => (Settings::default(), path, Some(format!("{}: {e}", l.display()))),
                },
                None => (Settings::default(), path, None),
            },
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }

    /// The legacy Python client's `~/ucLogger.json`.
    pub fn import_legacy(p: &Path) -> Result<Settings, String> {
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let buttons = |v: &serde_json::Value| -> Vec<Button> {
            v.as_array()
                .map(|a| a.iter().filter_map(|b| serde_json::from_value(b.clone()).ok()).collect())
                .unwrap_or_default()
        };
        let mut s = Settings::default();
        if let Some(n) = v["max_log_lines"].as_u64() {
            s.max_log_lines = n as usize;
        }
        if let Some(f) = v["file_path"].as_str() {
            s.files_dir = Some(f.into());
        }
        if let Some(f) = v["exportcsv_path"].as_str() {
            s.export_dir = Some(f.into());
        }
        let tb = buttons(&v["terminal"]["buttons"]);
        if !tb.is_empty() {
            s.terminal.buttons = tb;
        }
        let mut h: Vec<String> = v["terminal"]["history"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
        // Legacy history is most recent first.
        h.truncate(HISTORY_MAX);
        h.reverse();
        s.terminal.history = h;
        s.commands.buttons = buttons(&v["commands"]["buttons"]);
        for (k, c) in v["sources"].as_object().into_iter().flatten() {
            let ch = |n: &str| c[n].as_u64().unwrap_or(0).min(255) as u8;
            s.gui.log_colors.insert(k.clone(), [ch("r"), ch("g"), ch("b")]);
        }
        Ok(s)
    }

    /// Add a terminal history entry (deduplicated, most recent last).
    pub fn push_history(&mut self, line: &str) {
        if line.trim().is_empty() {
            return;
        }
        self.terminal.history.retain(|h| h != line);
        self.terminal.history.push(line.to_string());
        let n = self.terminal.history.len();
        if n > HISTORY_MAX {
            self.terminal.history.drain(..n - HISTORY_MAX);
        }
    }
}

/// The logdata token from the OS keyring.
pub fn keyring_token() -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_TOKEN).ok()?.get_password().ok()
}

/// Store (or with `None`, delete) the logdata token in the OS keyring.
pub fn set_keyring_token(token: Option<&str>) -> Result<(), String> {
    let e = keyring::Entry::new(KEYRING_SERVICE, KEYRING_TOKEN).map_err(|e| e.to_string())?;
    match token {
        Some(t) => e.set_password(t).map_err(|e| e.to_string()),
        None => match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_import() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ucLogger.json");
        std::fs::write(
            &p,
            r#"{"max_log_lines": 5000, "file_path": "/tmp/f",
                "terminal": {"buttons": [{"label": "Help", "cmd": "help"}, {"label": "Ver", "cmd": "version s"}],
                             "history": ["newest", "older", "oldest"]},
                "commands": {"buttons": [{"label": "Ping", "cmd": {"f": "cmd_ping"}}]},
                "sources": {"main": {"r": 175, "g": 82, "b": 101, "a": 60}}}"#,
        )
        .unwrap();
        let s = Settings::import_legacy(&p).unwrap();
        assert_eq!(s.max_log_lines, 5000);
        assert_eq!(s.files_dir.as_deref(), Some(Path::new("/tmp/f")));
        assert_eq!(s.terminal.buttons.len(), 2);
        assert_eq!(s.terminal.history, vec!["oldest", "older", "newest"]);
        assert_eq!(s.commands.buttons[0].cmd, serde_json::json!({"f": "cmd_ping"}));
        assert_eq!(s.gui.log_colors["main"], [175, 82, 101]);
    }

    #[test]
    fn round_trip_and_history() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("cfg/settings.json");
        let mut s = Settings::default();
        for i in 0..60 {
            s.push_history(&format!("cmd {i}"));
        }
        s.push_history("cmd 10");
        assert_eq!(s.terminal.history.len(), HISTORY_MAX);
        assert_eq!(s.terminal.history.last().unwrap(), "cmd 10");
        s.save(&p).unwrap();
        let (l, _, note) = Settings::load(Some(&p));
        assert_eq!(l, s);
        assert!(note.is_none());
        // Unknown and missing fields are tolerated.
        std::fs::write(&p, r#"{"max_log_lines": 7, "future": 1}"#).unwrap();
        assert_eq!(Settings::load(Some(&p)).0.max_log_lines, 7);
    }
}
