//! Front-end code shared by the TUI and the GUI: the log store, command buttons, the device list,
//! export names, and FW image suggestions.

pub mod logs;

use std::path::{Path, PathBuf};

use regex::Regex;
use uclog_link::{Device, Kind};
use uclog_proto::cbor::{self, Value};

use crate::settings::{self, Button, Settings};
use crate::LogdataArgs;

/// Session configuration from the command line, the settings and the keyring (in that order).
pub fn session_config(l: &LogdataArgs, files_dir: Option<PathBuf>, s: &Settings) -> uclog_session::Config {
    let mut resolver = l.resolver();
    if resolver.domain.is_none() {
        resolver.domain = s.logdata.domain.clone();
    }
    if l.logdata_url == crate::DEFAULT_LOGDATA_URL {
        if let Some(u) = &s.logdata.url {
            resolver.url = Some(u.clone());
        }
    }
    if resolver.token.is_none() && resolver.domain.is_some() {
        resolver.token = settings::keyring_token();
    }
    uclog_session::Config {
        resolver,
        semihost: uclog_session::SemihostConfig {
            files_dir: files_dir.or_else(|| s.files_dir.clone()),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The logdata token comes from the keyring (not `--logdata-token` / `$UCLOG_LOGDATA_TOKEN`).
pub fn token_from_keyring(l: &LogdataArgs) -> bool {
    l.logdata_token.is_none()
}

/// How to stop sending a token the service refuses (`Event::TokenRefused`), by where it came from.
pub fn stale_token_advice(l: &LogdataArgs) -> &'static str {
    if token_from_keyring(l) {
        "the account allows reading without a token: remove the saved token in the Settings of `uclog tui` or `uclog gui`"
    } else {
        "the account allows reading without a token: drop --logdata-token or unset UCLOG_LOGDATA_TOKEN"
    }
}

/// USB filters: the command line's, else the settings', else the defaults (empty).
pub fn usb_filters(cli: &[uclog_link::discover::UsbFilter], s: &Settings) -> Vec<uclog_link::discover::UsbFilter> {
    if !cli.is_empty() {
        return cli.to_vec();
    }
    s.usb_filters.iter().filter_map(|f| f.parse().ok()).collect()
}

pub type Answer = (String, String, String);

pub fn json_to_cbor(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i.into()),
            None => match n.as_u64() {
                Some(u) => Value::Integer(u.into()),
                None => Value::Float(n.as_f64().unwrap_or(0.0)),
            },
        },
        serde_json::Value::String(s) => Value::Text(s.clone()),
        serde_json::Value::Array(a) => Value::Array(a.iter().map(json_to_cbor).collect()),
        serde_json::Value::Object(o) => {
            Value::Map(o.iter().map(|(k, v)| (Value::Text(k.clone()), json_to_cbor(v))).collect())
        }
    }
}

/// `$name` placeholders in a button's map values: (key, placeholder).
pub fn button_args(b: &Button) -> Vec<(String, String)> {
    let re = Regex::new(r"\$\w+").unwrap();
    let mut out = vec![];
    if let serde_json::Value::Object(o) = &b.cmd {
        for (k, v) in o {
            if let Some(s) = v.as_str() {
                for m in re.find_iter(s) {
                    out.push((k.clone(), m.as_str().to_string()));
                }
            }
        }
    }
    out
}

/// The bytes a button sends on port 0 (legacy `UnitCommands.py` semantics): a string is a text
/// line; a map is CBOR, with `$name` placeholders replaced (a value that is a single
/// placeholder becomes a number when the answer parses as one).
pub fn button_payload(b: &Button, answers: &[Answer]) -> Vec<u8> {
    match &b.cmd {
        serde_json::Value::String(s) => format!("{s}\r").into_bytes(),
        serde_json::Value::Object(o) => {
            let re = Regex::new(r"\$\w+").unwrap();
            let mut m = o.clone();
            for (k, v) in o {
                let Some(s) = v.as_str() else { continue };
                let args: Vec<&str> = re.find_iter(s).map(|x| x.as_str()).collect();
                if args.is_empty() {
                    continue;
                }
                let val = |a: &str| {
                    answers.iter().find(|(kk, aa, _)| kk == k && aa == a).map(|x| x.2.clone()).unwrap_or_default()
                };
                let new = if args.len() == 1 && s == args[0] {
                    let a = val(args[0]);
                    if let Ok(i) = a.parse::<i64>() {
                        serde_json::json!(i)
                    } else if let Ok(f) = a.parse::<f64>() {
                        serde_json::json!(f)
                    } else {
                        serde_json::Value::String(a)
                    }
                } else {
                    let mut t = s.to_string();
                    for a in args {
                        t = t.replacen(a, &val(a), 1);
                    }
                    serde_json::Value::String(t)
                };
                m.insert(k.clone(), new);
            }
            cbor::to_vec(&json_to_cbor(&serde_json::Value::Object(m)))
        }
        other => cbor::to_vec(&json_to_cbor(other)),
    }
}

/// Remove ANSI escape sequences (for exports).
pub fn strip_ansi(s: &str) -> String {
    let re = Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07]*\x07").unwrap();
    re.replace_all(s, "").into_owned()
}

/// Hubs on this machine (registry, hub plan §7.1), as Connect-list entries.
pub fn local_hub_devices() -> Vec<Device> {
    uclog_hub::announce::local_hubs()
        .into_iter()
        .map(|j| {
            let s = |k: &str| j[k].as_str().unwrap_or("").to_string();
            Device {
                kind: Kind::Hub,
                uri: format!("tcp://localhost:{}", j["port"].as_u64().unwrap_or(9000)),
                name: s("name"),
                serial: s("serial_number"),
                description: format!("{} {} (local hub)", s("kind"), s("device")),
                busy: j["busy"].as_bool(),
            }
        })
        .collect()
}

/// `YYYYmmdd-HHMMSS` (UTC).
pub fn timestamp() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, secs) = (t.div_euclid(86400), t.rem_euclid(86400));
    // Civil date from days since 1970-01-01 (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

/// The working directory, or the home directory when it is `/` (ucLog.app started from the Finder).
pub fn start_dir() -> PathBuf {
    match std::env::current_dir() {
        Ok(d) if d.parent().is_some() => d,
        _ => directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()).unwrap_or_default(),
    }
}

/// `zephyr.signed.bin` files under `root` (build directories), at most `depth` levels down. None
/// from the home directory or above: that would read the user's folders, and macOS asks for each
/// protected one (Documents, Music, Photos, ...).
pub fn signed_images(root: &Path, depth: usize) -> Vec<PathBuf> {
    if directories::BaseDirs::new().is_some_and(|b| b.home_dir().starts_with(root)) {
        return vec![];
    }
    fn walk(d: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }
            if p.is_dir() {
                if depth > 0 {
                    walk(&p, depth - 1, out);
                }
            } else if name.ends_with(".signed.bin") {
                out.push(p);
            }
        }
    }
    let mut out = vec![];
    walk(root, depth, &mut out);
    out.sort();
    out.truncate(20);
    out
}

/// FW update progress as a fraction and a description.
pub fn fw_ratio(p: &uclog_session::FwProgress) -> (f64, String) {
    match p {
        uclog_session::FwProgress::Probing => (0.0, "probing port 59 / 3…".into()),
        uclog_session::FwProgress::Port(n) => (0.0, format!("using port {n}")),
        uclog_session::FwProgress::Starting => (0.0, "erasing (fw-start)…".into()),
        uclog_session::FwProgress::Block { done, total } => {
            (*done as f64 / (*total).max(1) as f64, format!("block {done}/{total}"))
        }
        uclog_session::FwProgress::Scheduling => (1.0, "scheduling…".into()),
        uclog_session::FwProgress::Done => (1.0, "done".into()),
    }
}
