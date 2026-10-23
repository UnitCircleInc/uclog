//! `uclog list`

use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use uclog_link::discover::{self, UsbFilter};
use uclog_link::Device;

#[derive(Args)]
pub struct ListArgs {
    /// One JSON array on stdout
    #[arg(long)]
    json: bool,
    /// Also scan for BLE devices for this many seconds
    #[arg(long, value_name = "SECS")]
    ble_scan: Option<f64>,
    /// Browse mDNS for hubs for this many seconds (0 = skip)
    #[arg(long, value_name = "SECS", default_value_t = 1.0)]
    mdns: f64,
    /// USB serial filter VID:PID; repeatable; replaces the defaults
    #[arg(long = "usb-filter", value_name = "VID:PID")]
    usb_filter: Vec<UsbFilter>,
}

pub async fn run(a: ListArgs) -> anyhow::Result<ExitCode> {
    let mut all: Vec<Device> = discover::local_devices(&a.usb_filter);
    let mut notes = vec![];
    if let Some(s) = a.ble_scan {
        match discover::ble_devices(Duration::from_secs_f64(s)).await {
            Ok(v) => all.extend(v),
            Err(e) => notes.push(format!("BLE scan: {e}")),
        }
    }
    // Hubs on this machine (registry, hub plan §7.1), then remote ones (mDNS).
    let local: Vec<Device> = uclog_hub::announce::local_hubs()
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
    if a.mdns > 0.0 {
        match discover::hubs(Duration::from_secs_f64(a.mdns)).await {
            // A local hub also shows up over mDNS: list it once.
            Ok(v) => all.extend(v.into_iter().filter(|d| !local_names.contains(&d.name))),
            Err(e) => notes.push(format!("mDNS: {e}")),
        }
    }
    if a.json {
        println!("[{}]", all.iter().map(Device::to_json).collect::<Vec<_>>().join(","));
    } else if all.is_empty() {
        println!("no devices found");
    } else {
        println!("{:<6} {:<44} {:<28} {:<24} DESCRIPTION", "KIND", "URI", "NAME", "SERIAL");
        for d in &all {
            let busy = if d.busy == Some(true) { " [busy]" } else { "" };
            println!("{:<6} {:<44} {:<28} {:<24} {}{busy}", d.kind.as_str(), d.uri, d.name, d.serial, d.description);
        }
    }
    for n in notes {
        eprintln!("uclog: {n}");
    }
    Ok(ExitCode::SUCCESS)
}
