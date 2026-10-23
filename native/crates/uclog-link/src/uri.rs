//! Device URIs (plans README §5.7):
//! `serial:/dev/ttyACM0`, `serial:COM3`, `hid:<path or serial>`, `ble:<address or name>`,
//! `udp://host:port`, `tcp://host[:9000]`, `ws(s)://host[:9000][/path]`, `auto`.

use std::fmt;
use std::str::FromStr;

use uclog_proto::link::{DEFAULT_BAUD, DEFAULT_HUB_PORT};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkSpec {
    Serial { path: String, baud: u32 },
    Hid { id: String },
    Ble { id: String },
    Udp { addr: String },
    Tcp { addr: String },
    Ws { url: String },
    Auto,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid device URI {uri:?}: {msg}")]
pub struct UriError {
    pub uri: String,
    pub msg: String,
}

fn with_port(hostport: &str, default: Option<u16>) -> Option<String> {
    if hostport.is_empty() {
        return None;
    }
    // [v6]:port, host:port or host
    let has_port = if hostport.starts_with('[') { hostport.contains("]:") } else { hostport.matches(':').count() == 1 };
    if has_port {
        Some(hostport.to_string())
    } else {
        default.map(|p| {
            if hostport.contains(':') && !hostport.starts_with('[') {
                format!("[{hostport}]:{p}")
            } else {
                format!("{hostport}:{p}")
            }
        })
    }
}

impl FromStr for LinkSpec {
    type Err = UriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = |m: &str| UriError { uri: s.into(), msg: m.into() };
        if s == "auto" {
            return Ok(LinkSpec::Auto);
        }
        if let Some(r) = s.strip_prefix("udp://") {
            return Ok(LinkSpec::Udp { addr: with_port(r, None).ok_or_else(|| err("UDP needs host:port"))? });
        }
        if let Some(r) = s.strip_prefix("tcp://") {
            return Ok(LinkSpec::Tcp {
                addr: with_port(r, Some(DEFAULT_HUB_PORT)).ok_or_else(|| err("missing host"))?,
            });
        }
        if s.starts_with("ws://") || s.starts_with("wss://") {
            let (scheme, rest) = s.split_once("://").unwrap();
            let (hp, path) = rest.split_once('/').map(|(a, b)| (a, format!("/{b}"))).unwrap_or((rest, "/".into()));
            let hp = with_port(hp, Some(DEFAULT_HUB_PORT)).ok_or_else(|| err("missing host"))?;
            return Ok(LinkSpec::Ws { url: format!("{scheme}://{hp}{path}") });
        }
        if let Some((kind, rest)) = s.split_once(':') {
            if rest.is_empty() {
                return Err(err("missing device"));
            }
            return match kind {
                "serial" => Ok(LinkSpec::Serial { path: rest.into(), baud: DEFAULT_BAUD }),
                "hid" => Ok(LinkSpec::Hid { id: rest.into() }),
                "ble" => Ok(LinkSpec::Ble { id: rest.into() }),
                _ => Err(err("unknown scheme (serial:, hid:, ble:, udp://, tcp://, ws://, wss://, auto)")),
            };
        }
        Err(err("unknown scheme (serial:, hid:, ble:, udp://, tcp://, ws://, wss://, auto)"))
    }
}

impl fmt::Display for LinkSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkSpec::Serial { path, .. } => write!(f, "serial:{path}"),
            LinkSpec::Hid { id } => write!(f, "hid:{id}"),
            LinkSpec::Ble { id } => write!(f, "ble:{id}"),
            LinkSpec::Udp { addr } => write!(f, "udp://{addr}"),
            LinkSpec::Tcp { addr } => write!(f, "tcp://{addr}"),
            LinkSpec::Ws { url } => write!(f, "{url}"),
            LinkSpec::Auto => write!(f, "auto"),
        }
    }
}

impl LinkSpec {
    /// Set the baud rate (serial only).
    pub fn with_baud(self, b: u32) -> Self {
        match self {
            LinkSpec::Serial { path, .. } => LinkSpec::Serial { path, baud: b },
            o => o,
        }
    }

    /// True for links to a hub rather than a device.
    pub fn is_hub(&self) -> bool {
        matches!(self, LinkSpec::Tcp { .. } | LinkSpec::Ws { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let p = |s: &str| s.parse::<LinkSpec>().unwrap();
        assert_eq!(p("serial:/dev/ttyACM0"), LinkSpec::Serial { path: "/dev/ttyACM0".into(), baud: 1_000_000 });
        assert_eq!(p("serial:COM3"), LinkSpec::Serial { path: "COM3".into(), baud: 1_000_000 });
        assert_eq!(p("hid:0042AB"), LinkSpec::Hid { id: "0042AB".into() });
        assert_eq!(p("tcp://labpc"), LinkSpec::Tcp { addr: "labpc:9000".into() });
        assert_eq!(p("tcp://10.0.0.2:9001"), LinkSpec::Tcp { addr: "10.0.0.2:9001".into() });
        assert_eq!(p("tcp://::1"), LinkSpec::Tcp { addr: "[::1]:9000".into() });
        assert_eq!(p("ws://localhost"), LinkSpec::Ws { url: "ws://localhost:9000/".into() });
        assert_eq!(p("wss://h:443/uclog"), LinkSpec::Ws { url: "wss://h:443/uclog".into() });
        assert_eq!(p("udp://10.0.0.5:4000"), LinkSpec::Udp { addr: "10.0.0.5:4000".into() });
        assert_eq!(p("auto"), LinkSpec::Auto);
        assert!("udp://h".parse::<LinkSpec>().is_err());
        assert!("foo:bar".parse::<LinkSpec>().is_err());
        assert!("serial:".parse::<LinkSpec>().is_err());
        for s in ["serial:/dev/x", "hid:a", "ble:b", "udp://h:1", "tcp://h:2", "ws://h:3/"] {
            assert_eq!(p(s).to_string(), s);
        }
    }
}
