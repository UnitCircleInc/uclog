//! Device and hub discovery (plans README §5.1).

use std::time::Duration;

use serde_like::Serializeish;
use uclog_proto::link::SERIAL_FILTER;

use crate::{LinkError, LinkSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Serial,
    Hid,
    Ble,
    Hub,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Serial => "serial",
            Kind::Hid => "hid",
            Kind::Ble => "ble",
            Kind::Hub => "hub",
        }
    }
}

/// One entry of the device list every client shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub kind: Kind,
    pub uri: String,
    pub name: String,
    pub serial: String,
    pub description: String,
    /// Hubs only: another client is connected.
    pub busy: Option<bool>,
}

impl Device {
    /// JSON object (no serde dependency in this crate).
    pub fn to_json(&self) -> String {
        Serializeish::object(&[
            ("kind", Serializeish::str(self.kind.as_str())),
            ("uri", Serializeish::str(&self.uri)),
            ("name", Serializeish::str(&self.name)),
            ("serial", Serializeish::str(&self.serial)),
            ("description", Serializeish::str(&self.description)),
            ("busy", self.busy.map(|b| b.to_string()).unwrap_or_else(|| "null".into())),
        ])
    }
}

/// A USB VID:PID filter entry; `pid == None` matches any PID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbFilter {
    pub vid: u16,
    pub pid: Option<u16>,
}

impl std::str::FromStr for UsbFilter {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let (v, p) = s.split_once(':').ok_or_else(|| format!("expected VID:PID, got {s:?}"))?;
        let hex = |x: &str| u16::from_str_radix(x, 16).map_err(|_| format!("bad hex {x:?} in {s:?}"));
        Ok(UsbFilter { vid: hex(v)?, pid: if p == "*" { None } else { Some(hex(p)?) } })
    }
}

pub fn default_filters() -> Vec<UsbFilter> {
    SERIAL_FILTER.iter().map(|(vid, pid, _)| UsbFilter { vid: *vid, pid: *pid }).collect()
}

fn family(vid: u16, pid: u16) -> &'static str {
    SERIAL_FILTER
        .iter()
        .find(|(v, p, _)| *v == vid && p.is_none_or(|p| p == pid))
        .map(|(_, _, n)| *n)
        .unwrap_or("USB serial")
}

/// Serial ports matching `filters` (the default filter when empty).
pub fn serial_ports(filters: &[UsbFilter]) -> Vec<Device> {
    let defaults = default_filters();
    let filters = if filters.is_empty() { &defaults[..] } else { filters };
    let mut out = vec![];
    for p in serialport::available_ports().unwrap_or_default() {
        // macOS lists each port twice; the tty.* node blocks on DCD, use cu.*.
        if cfg!(target_os = "macos") && p.port_name.starts_with("/dev/tty.") {
            continue;
        }
        let serialport::SerialPortType::UsbPort(u) = &p.port_type else { continue };
        if !filters.iter().any(|f| f.vid == u.vid && f.pid.is_none_or(|x| x == u.pid)) {
            continue;
        }
        out.push(Device {
            kind: Kind::Serial,
            uri: format!("serial:{}", p.port_name),
            name: u.product.clone().unwrap_or_else(|| family(u.vid, u.pid).into()),
            serial: u.serial_number.clone().unwrap_or_default(),
            description: format!("{:04x}:{:04x} {}", u.vid, u.pid, u.manufacturer.clone().unwrap_or_default())
                .trim_end()
                .into(),
            busy: None,
        });
    }
    out.sort_by(|a, b| a.uri.cmp(&b.uri));
    out
}

pub fn hid_devices() -> Vec<Device> {
    crate::hid::list()
}

pub async fn ble_devices(scan: Duration) -> Result<Vec<Device>, LinkError> {
    Ok(crate::ble::scan(scan).await?.into_iter().map(|(_, d)| d).collect())
}

/// Hubs advertised over mDNS (`_uclog._tcp.local.`).
pub async fn hubs(browse: Duration) -> Result<Vec<Device>, LinkError> {
    tokio::task::spawn_blocking(move || {
        let d = mdns_sd::ServiceDaemon::new().map_err(|e| LinkError::NotFound(format!("mDNS: {e}")))?;
        let rx = d.browse("_uclog._tcp.local.").map_err(|e| LinkError::NotFound(format!("mDNS: {e}")))?;
        let deadline = std::time::Instant::now() + browse;
        let mut out: Vec<Device> = vec![];
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            let Ok(ev) = rx.recv_timeout(left) else { break };
            if let mdns_sd::ServiceEvent::ServiceResolved(s) = ev {
                let Some(ip) = s.get_addresses_v4().into_iter().next() else { continue };
                let instance = s.get_fullname().trim_end_matches("._uclog._tcp.local.").to_string();
                let uri = format!("tcp://{ip}:{}", s.get_port());
                if out.iter().any(|x| x.uri == uri) {
                    continue;
                }
                let txt = |k: &str| s.get_property_val_str(k).unwrap_or("").to_string();
                out.push(Device {
                    kind: Kind::Hub,
                    uri,
                    name: instance,
                    serial: txt("serial_number"),
                    description: [txt("kind"), txt("description")].join(" ").trim().into(),
                    busy: Some(txt("busy") == "1"),
                });
            }
        }
        let _ = d.shutdown();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    })
    .await
    .map_err(|e| LinkError::NotFound(e.to_string()))?
}

/// Local USB devices (serial + HID), the candidates for `auto`.
pub fn local_devices(filters: &[UsbFilter]) -> Vec<Device> {
    let mut v = serial_ports(filters);
    v.extend(hid_devices());
    v
}

/// Resolve `auto`: exactly one local USB device. Otherwise an error listing the candidates.
pub fn resolve_auto(filters: &[UsbFilter]) -> Result<LinkSpec, (String, Vec<Device>)> {
    let c = local_devices(filters);
    match c.len() {
        1 => c[0].uri.parse().map_err(|e: crate::UriError| (e.to_string(), c.clone())),
        0 => Err(("no device found (serial filter + HID); use -d to choose one".into(), c)),
        n => Err((format!("{n} devices found; choose one with -d"), c)),
    }
}

/// Minimal JSON writing without pulling serde into this crate.
mod serde_like {
    pub struct Serializeish;

    impl Serializeish {
        pub fn str(s: &str) -> String {
            let mut o = String::from("\"");
            for c in s.chars() {
                match c {
                    '"' => o.push_str("\\\""),
                    '\\' => o.push_str("\\\\"),
                    '\n' => o.push_str("\\n"),
                    c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                    c => o.push(c),
                }
            }
            o.push('"');
            o
        }

        pub fn object(fields: &[(&str, String)]) -> String {
            format!("{{{}}}", fields.iter().map(|(k, v)| format!("{}:{v}", Self::str(k))).collect::<Vec<_>>().join(","))
        }
    }
}
