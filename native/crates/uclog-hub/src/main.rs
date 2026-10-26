//! `uclog-hub`: the hub on its own, as a small executable (`uclog hub` is the same thing).

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "uclog-hub",
    version = env!("UCLOG_VERSION"),
    about = "ucLog hub: one device, one TCP/WebSocket/UDP client at a time",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// -v: statistics every 10 s, -vv: debug
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(flatten)]
    hub: uclog_hub::HubArgs,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// List local USB devices (serial and HID), for programs that only have `uclog-hub` (the
    /// Python library's wheels, the VS Code extension). Hubs and BLE devices on request.
    List {
        /// One JSON array on stdout
        #[arg(long)]
        json: bool,
        /// Also scan for BLE devices for this many seconds
        #[arg(long, value_name = "SECS")]
        ble_scan: Option<f64>,
        /// Also list hubs: the ones on this machine, and those found over mDNS within SECS (0 = this
        /// machine only)
        #[arg(long, value_name = "SECS")]
        hubs: Option<f64>,
        /// USB serial filter VID:PID; repeatable; replaces the defaults
        #[arg(long = "usb-filter", value_name = "VID:PID")]
        usb_filter: Vec<uclog_link::discover::UsbFilter>,
    },
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    if let Some(Cmd::List { json, ble_scan, hubs, usb_filter }) = &cli.cmd {
        let secs = |s: &Option<f64>| s.map(|s| std::time::Duration::from_secs_f64(s.max(0.0)));
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let (all, notes) = rt.block_on(uclog_hub::list::devices(usb_filter, secs(ble_scan), secs(hubs)));
        uclog_hub::list::print(&all, *json);
        for n in notes {
            eprintln!("uclog-hub: {n}");
        }
        return std::process::ExitCode::SUCCESS;
    }
    let level = if cli.verbose >= 2 { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| level.into()))
        .with_writer(std::io::stderr)
        // A failed write would be reported with eprintln!, which panics (and aborts) when stderr is
        // gone, e.g. when the script that started the hub was killed.
        .log_internal_errors(false)
        .init();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let r = rt.block_on(uclog_hub::run(cli.hub, cli.verbose));
    rt.shutdown_timeout(std::time::Duration::from_millis(200));
    match r {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("uclog-hub: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
