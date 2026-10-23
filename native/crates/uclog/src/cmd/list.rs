//! `uclog list`

use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use uclog_link::discover::UsbFilter;

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
    let (all, notes) = uclog_hub::list::devices(
        &a.usb_filter,
        a.ble_scan.map(Duration::from_secs_f64),
        Some(Duration::from_secs_f64(a.mdns)),
    )
    .await;
    uclog_hub::list::print(&all, a.json);
    for n in notes {
        eprintln!("uclog: {n}");
    }
    Ok(ExitCode::SUCCESS)
}
