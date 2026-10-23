//! `uclog-hub`: the hub on its own, as a small executable (`uclog hub` is the same thing).

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "uclog-hub",
    version,
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
    /// List local USB devices (serial and HID), for scripts that only have `uclog-hub` (the Python
    /// library's wheels). `uclog list` also shows hubs and BLE devices.
    List {
        /// One JSON array on stdout
        #[arg(long)]
        json: bool,
        /// USB serial filter VID:PID; repeatable; replaces the defaults
        #[arg(long = "usb-filter", value_name = "VID:PID")]
        usb_filter: Vec<uclog_link::discover::UsbFilter>,
    },
}

fn list(json: bool, filters: &[uclog_link::discover::UsbFilter]) {
    let all = uclog_link::discover::local_devices(filters);
    if json {
        println!("[{}]", all.iter().map(uclog_link::Device::to_json).collect::<Vec<_>>().join(","));
    } else if all.is_empty() {
        println!("no devices found");
    } else {
        for d in &all {
            println!("{:<6} {:<44} {:<28} {:<24} {}", d.kind.as_str(), d.uri, d.name, d.serial, d.description);
        }
    }
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    if let Some(Cmd::List { json, usb_filter }) = &cli.cmd {
        list(*json, usb_filter);
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
