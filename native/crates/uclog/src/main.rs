//! `uclog`: ucLog client tools (list, decode, term, tui, fw-update, hub).
//! `gui` arrives in a later phase (plans README §6).

mod cmd;
mod settings;
mod tui;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use uclog_link::discover::UsbFilter;
use uclog_link::LinkSpec;

#[derive(Parser)]
#[command(name = "uclog", version, about = "ucLog client tools", propagate_version = true)]
struct Cli {
    /// -v: info, -vv: debug + wire trace
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List serial/HID devices, BLE devices and hubs.
    List(cmd::list::ListArgs),
    /// Decode a raw wire capture (or stdin) to log lines.
    Decode(cmd::decode::DecodeArgs),
    /// Interactive terminal on port 61 (or another port); replaces logcmd.py.
    Term(cmd::term::TermArgs),
    /// Terminal UI: logs, terminal, commands, device info, FW update; replaces uclog-cli.py.
    Tui(tui::TuiArgs),
    /// Update device firmware with a signed image (port 59, falling back to port 3).
    FwUpdate(cmd::fw::FwArgs),
    /// Share one device with one TCP/WebSocket/UDP client at a time (also `uclog-hub`).
    Hub(uclog_hub::HubArgs),
}

/// Device selection, shared by subcommands that open a device.
#[derive(Args, Clone, Debug)]
pub struct DeviceArgs {
    /// serial:PATH, hid:PATH|SERIAL, ble:ADDR|NAME, udp://HOST:PORT, tcp://HOST[:9000],
    /// ws(s)://HOST[:9000], or auto (exactly one local USB device)
    #[arg(short, long, default_value = "auto", env = "UCLOG_DEVICE")]
    pub device: LinkSpec,
    /// Serial baud rate
    #[arg(long, default_value_t = uclog_proto::link::DEFAULT_BAUD)]
    pub baud: u32,
    /// USB serial filter VID:PID (hex, PID may be *); repeatable; replaces the defaults
    #[arg(long = "usb-filter", value_name = "VID:PID")]
    pub usb_filter: Vec<UsbFilter>,
}

/// Logdata resolution, shared by subcommands that decode logs.
#[derive(Args, Clone, Debug, Default)]
pub struct LogdataArgs {
    /// Logdata file (.logdata or ELF with .logdata_cbor); repeatable
    #[arg(short = 'l', long = "logdata", value_name = "FILE")]
    pub logdata: Vec<PathBuf>,
    /// Logdata cache directory [default: platform cache dir]
    #[arg(long, value_name = "DIR")]
    pub cache_dir: Option<PathBuf>,
    /// Domain for remote logdata lookup
    #[arg(long, env = "UCLOG_LOGDATA_DOMAIN")]
    pub logdata_domain: Option<String>,
    /// Bearer token for remote logdata lookup
    #[arg(long, env = "UCLOG_LOGDATA_TOKEN", hide_env_values = true)]
    pub logdata_token: Option<String>,
    /// Logdata service URL
    #[arg(long, env = "UCLOG_LOGDATA_URL", default_value = DEFAULT_LOGDATA_URL)]
    pub logdata_url: String,
}

impl LogdataArgs {
    pub fn resolver(&self) -> uclog_session::ResolverConfig {
        uclog_session::ResolverConfig {
            manual: self.logdata.clone(),
            cache_dir: self.cache_dir.clone().or_else(default_cache_dir),
            domain: self.logdata_domain.clone(),
            token: self.logdata_token.clone(),
            url: Some(self.logdata_url.clone()),
            images: vec![],
        }
    }
}

pub const DEFAULT_LOGDATA_URL: &str = "https://fw.unitcircle.ca";

pub fn default_cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("ca", "unitcircle", "uclog").map(|d| d.cache_dir().join("logdata"))
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    // The hub reports connections at info level; the client tools are quiet by default.
    let is_hub = matches!(cli.cmd, Command::Hub(_));
    let level = match cli.verbose {
        0 if is_hub => "info",
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| level.into()))
        .with_writer(std::io::stderr)
        // A failed write would be reported with eprintln!, which panics (and aborts) when stderr is
        // gone, e.g. when the script that started the hub was killed.
        .log_internal_errors(false)
        .init();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let r = rt.block_on(async {
        match cli.cmd {
            Command::List(a) => cmd::list::run(a).await,
            Command::Decode(a) => cmd::decode::run(a).await,
            Command::Term(a) => cmd::term::run(a).await,
            Command::Tui(a) => tui::run(a).await,
            Command::FwUpdate(a) => cmd::fw::run(a).await,
            Command::Hub(a) => uclog_hub::run(a, cli.verbose).await.map(|_| std::process::ExitCode::SUCCESS),
        }
    });
    // Background threads (serial/HID readers) must not keep the process alive.
    rt.shutdown_timeout(std::time::Duration::from_millis(200));
    match r {
        Ok(code) => code,
        Err(e) => {
            eprintln!("uclog: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
