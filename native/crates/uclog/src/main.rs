//! `uclog`: ucLog client tools (list, decode, term, tui, gui, fw-update, hub).

mod cmd;
mod front;
#[cfg(feature = "gui")]
mod gui;
mod settings;
mod tui;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use uclog_link::discover::UsbFilter;
use uclog_link::LinkSpec;

#[derive(Parser)]
#[command(name = "uclog", version = env!("UCLOG_VERSION"), about = "ucLog client tools", propagate_version = true)]
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
    /// Graphical client: logs, terminal, commands, plots, device info, FW update; replaces uclog-gui.py.
    #[cfg(feature = "gui")]
    Gui(gui::GuiArgs),
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
    /// Logdata cache directory [default: $XDG_CACHE_HOME/uclog, else ~/.cache/uclog]
    #[arg(long, value_name = "DIR", env = "UCLOG_CACHE_DIR")]
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

/// `$XDG_CACHE_HOME/uclog`, else `~/.cache/uclog`, on every platform: the cache the device builds
/// write `<app_hash>.logdata` into (the firmware Makefiles, the Zephyr module's `cachelogdata.py`).
pub fn default_cache_dir() -> Option<PathBuf> {
    match std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        Some(x) => Some(PathBuf::from(x).join("uclog")),
        None => directories::BaseDirs::new().map(|d| d.home_dir().join(".cache/uclog")),
    }
}

/// The arguments; with none, from ucLog.app (Finder, the Dock: packaging/macos/release.sh), `gui`.
fn args() -> Vec<std::ffi::OsString> {
    #[allow(unused_mut)]
    let mut a: Vec<_> = std::env::args_os().collect();
    #[cfg(all(target_os = "macos", feature = "gui"))]
    if a.len() == 1 && std::env::current_exe().is_ok_and(|p| p.parent().is_some_and(|d| d.ends_with("Contents/MacOS"))) {
        a.push("gui".into());
    }
    a
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse_from(args());
    // The hub reports connections at info level; the client tools are quiet by default.
    let is_hub = matches!(cli.cmd, Command::Hub(_));
    // btleplug logs at error level for things it then reports as Ok (CoreBluetooth: disconnecting a
    // peripheral that is already gone); its real failures come back as errors from its calls.
    // winit warns on every cursor change when Wayland has no cursor theme (WSLg, minimal installs).
    const QUIET: &str = "btleplug=off,winit::platform_impl::linux::wayland::window::state=error";
    let level = match cli.verbose {
        0 if is_hub => format!("info,{QUIET}"),
        0 => format!("warn,{QUIET}"),
        1 => "info,btleplug=off".to_string(),
        _ => "debug".to_string(),
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
            // The GUI owns the main thread (macOS requires it); its session runs on the runtime.
            #[cfg(feature = "gui")]
            Command::Gui(a) => tokio::task::block_in_place(|| {
                let _g = tokio::runtime::Handle::current().enter();
                gui::run(a)
            }),
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
