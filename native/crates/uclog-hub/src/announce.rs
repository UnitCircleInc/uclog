//! Telling clients where the hub is: mDNS (hub plan §7) and the local registry file (§7.1).

use std::collections::HashMap;
use std::path::PathBuf;

use mdns_sd::{ServiceDaemon, ServiceInfo};
use serde_json::json;
use tracing::{debug, warn};

/// What the hub serves.
#[derive(Debug, Clone)]
pub struct Identity {
    /// mDNS instance name: `{hostname}-{interface}`.
    pub name: String,
    pub interface: String,
    pub kind: String,
    pub device: String,
    pub serial_number: String,
    pub description: String,
    pub port: u16,
    pub ws_port: Option<u16>,
    pub udp_port: Option<u16>,
    pub bind: String,
    pub tls: bool,
}

/// `ttyACM0`, `COM3`, `hid-<serial>`, `ble-<addr>`, `udp-<ip>`, sanitised.
pub fn interface_name(spec: &uclog_link::LinkSpec) -> String {
    use uclog_link::LinkSpec as L;
    let raw = match spec {
        L::Serial { path, .. } => path.rsplit(['/', '\\']).next().unwrap_or(path).trim_start_matches("cu.").to_string(),
        L::Hid { id } => format!("hid-{}", id.rsplit('/').next().unwrap_or(id)),
        L::Ble { id } => format!("ble-{id}"),
        L::Udp { addr } => format!("udp-{}", addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr)),
        L::Loopback { .. } => "loopback".into(),
        other => other.to_string(),
    };
    raw.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

pub fn hostname() -> String {
    let h = gethostname::gethostname().to_string_lossy().into_owned();
    h.split('.').next().unwrap_or(&h).to_string()
}

pub struct Mdns {
    daemon: ServiceDaemon,
    id: Identity,
    fullnames: Vec<String>,
}

impl Mdns {
    pub fn start(id: Identity) -> Option<Self> {
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                warn!("mDNS disabled: {e}");
                return None;
            }
        };
        let mut m = Mdns { daemon, id, fullnames: vec![] };
        m.publish(false);
        Some(m)
    }

    fn props(&self, busy: bool) -> HashMap<String, String> {
        let id = &self.id;
        HashMap::from([
            ("serial_number".into(), id.serial_number.clone()),
            ("description".into(), id.description.clone()),
            ("interface".into(), id.interface.clone()),
            ("kind".into(), id.kind.clone()),
            ("proto".into(), "1".into()),
            ("ws".into(), id.ws_port.map(|p| p.to_string()).unwrap_or_else(|| "same".into())),
            ("tls".into(), (id.tls as u8).to_string()),
            ("busy".into(), (busy as u8).to_string()),
        ])
    }

    /// (Re-)register; mdns-sd replaces the TXT record of an existing registration.
    pub fn publish(&mut self, busy: bool) {
        let host = format!("{}.local.", hostname());
        let name: String = self.id.name.chars().take(63).collect();
        let mut services = vec![("_uclog._tcp.local.", self.id.port)];
        if let Some(p) = self.id.udp_port {
            services.push(("_uclog._udp.local.", p));
        }
        self.fullnames.clear();
        for (ty, port) in services {
            match ServiceInfo::new(ty, &name, &host, "", port, self.props(busy)) {
                Ok(info) => {
                    let info = info.enable_addr_auto();
                    self.fullnames.push(info.get_fullname().to_string());
                    if let Err(e) = self.daemon.register(info) {
                        warn!("mDNS register: {e}");
                    }
                }
                Err(e) => warn!("mDNS: {e}"),
            }
        }
        debug!("mDNS: {name} busy={busy}");
    }

    pub fn stop(self) {
        for f in &self.fullnames {
            if let Ok(rx) = self.daemon.unregister(f) {
                let _ = rx.recv_timeout(std::time::Duration::from_millis(500));
            }
        }
        let _ = self.daemon.shutdown();
    }
}

/// `<runtime dir>/uclog/hubs` (hub plan §7.1).
pub fn registry_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Some(PathBuf::from(d).join("uclog/hubs"));
    }
    #[cfg(windows)]
    if let Some(d) = std::env::var_os("LOCALAPPDATA") {
        return Some(PathBuf::from(d).join("uclog\\run\\hubs"));
    }
    directories::ProjectDirs::from("ca", "unitcircle", "uclog").map(|d| d.cache_dir().join("run/hubs"))
}

/// The registry file of this hub; removed on drop.
pub struct Registry {
    path: PathBuf,
    id: Identity,
}

impl Registry {
    pub fn create(id: Identity) -> Option<Self> {
        let dir = registry_dir()?;
        if let Err(e) = std::fs::create_dir_all(&dir) {
            warn!("registry {}: {e}", dir.display());
            return None;
        }
        let r = Registry { path: dir.join(format!("{}.json", std::process::id())), id };
        r.write(false);
        Some(r)
    }

    pub fn write(&self, busy: bool) {
        let id = &self.id;
        let v = json!({
            "pid": std::process::id(), "port": id.port, "ws_port": id.ws_port, "udp_port": id.udp_port,
            "bind": id.bind, "name": id.name, "device": id.device, "serial_number": id.serial_number,
            "kind": id.kind, "tls": id.tls, "busy": busy, "version": env!("CARGO_PKG_VERSION"),
        });
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, v.to_string()).and_then(|_| std::fs::rename(&tmp, &self.path)).is_err() {
            warn!("could not write {}", self.path.display());
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Hubs in the local registry whose process is still running.
pub fn local_hubs() -> Vec<serde_json::Value> {
    let Some(dir) = registry_dir() else { return vec![] };
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let mut v = vec![];
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(j) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        else {
            continue;
        };
        let pid = j["pid"].as_u64().unwrap_or(0) as u32;
        if pid_alive(pid) {
            v.push(j);
        } else {
            let _ = std::fs::remove_file(&p); // crash leftovers
        }
    }
    v
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    pid != 0
        && (unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}

#[cfg(not(unix))]
fn pid_alive(pid: u32) -> bool {
    pid != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use uclog_link::LinkSpec;

    #[test]
    fn interface_names() {
        let n = |s: &str| interface_name(&s.parse::<LinkSpec>().unwrap());
        assert_eq!(n("serial:/dev/ttyACM0"), "ttyACM0");
        assert_eq!(n("serial:/dev/cu.usbmodem113103"), "usbmodem113103");
        assert_eq!(n("serial:COM3"), "COM3");
        assert_eq!(n("ble:AA:BB:CC:DD:EE:FF"), "ble-AA-BB-CC-DD-EE-FF");
        assert_eq!(n("udp://10.0.0.5:4000"), "udp-10-0-0-5");
        assert_eq!(n("loopback:rate=1"), "loopback");
    }
}
