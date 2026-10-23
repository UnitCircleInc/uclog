//! The device list shared by `uclog list` and `uclog-hub list` (USB, optionally BLE and hubs).

use std::time::Duration;

use uclog_link::discover::{self, UsbFilter};
use uclog_link::Device;

/// Local USB devices; BLE devices when `ble_scan` is set; with `hubs`, the hubs on this machine
/// (registry) and, for a non-zero duration, the ones found over mDNS. Problems come back as notes.
pub async fn devices(
    filters: &[UsbFilter],
    ble_scan: Option<Duration>,
    hubs: Option<Duration>,
) -> (Vec<Device>, Vec<String>) {
    let mut all: Vec<Device> = discover::local_devices(filters);
    let mut notes = vec![];
    if let Some(s) = ble_scan {
        match discover::ble_devices(s).await {
            Ok(v) => all.extend(v),
            Err(e) => notes.push(format!("BLE scan: {e}")),
        }
    }
    let Some(browse) = hubs else { return (all, notes) };
    // Hubs on this machine (registry, hub plan §7.1), then remote ones (mDNS).
    let local: Vec<Device> = crate::announce::local_hubs()
        .into_iter()
        .map(|j| {
            let s = |k: &str| j[k].as_str().unwrap_or("").to_string();
            Device {
                kind: discover::Kind::Hub,
                uri: format!("tcp://localhost:{}", j["port"].as_u64().unwrap_or(9000)),
                name: s("name"),
                serial: s("serial_number"),
                description: format!("{} {} (local hub)", s("kind"), s("device")),
                busy: j["busy"].as_bool(),
            }
        })
        .collect();
    let local_names: Vec<String> = local.iter().map(|d| d.name.clone()).collect();
    all.extend(local);
    if !browse.is_zero() {
        match discover::hubs(browse).await {
            // A local hub also shows up over mDNS: list it once.
            Ok(v) => all.extend(v.into_iter().filter(|d| !local_names.contains(&d.name))),
            Err(e) => notes.push(format!("mDNS: {e}")),
        }
    }
    (all, notes)
}

/// The table `uclog list` and `uclog-hub list` print without `--json`.
pub fn print(all: &[Device], json: bool) {
    if json {
        println!("[{}]", all.iter().map(Device::to_json).collect::<Vec<_>>().join(","));
    } else if all.is_empty() {
        println!("no devices found");
    } else {
        println!("{:<6} {:<44} {:<28} {:<24} DESCRIPTION", "KIND", "URI", "NAME", "SERIAL");
        for d in all {
            let busy = if d.busy == Some(true) { " [busy]" } else { "" };
            println!("{:<6} {:<44} {:<28} {:<24} {}{busy}", d.kind.as_str(), d.uri, d.name, d.serial, d.description);
        }
    }
}
