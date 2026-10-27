//! Hub command line (hub plan §8), shared by `uclog hub` and `uclog-hub`.

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, ValueEnum};
use uclog_link::discover::UsbFilter;
use uclog_link::LinkSpec;

use crate::admission::Cidr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SlowClient {
    /// Skip to the oldest frame still held and count the dropped frames
    Drop,
    /// End the session
    Disconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

#[derive(Args, Debug, Clone)]
pub struct HubArgs {
    /// serial:PATH, hid:PATH|SERIAL, ble:ADDR|NAME, udp://HOST:PORT, or auto (exactly one local USB device)
    #[arg(short, long, default_value = "auto", env = "UCLOG_DEVICE")]
    pub device: LinkSpec,
    /// Serial baud rate
    #[arg(long, default_value_t = uclog_proto::link::DEFAULT_BAUD)]
    pub baud: u32,
    /// USB serial filter VID:PID (hex, PID may be *); repeatable; replaces the defaults
    #[arg(long = "usb-filter", value_name = "VID:PID")]
    pub usb_filter: Vec<UsbFilter>,

    /// TCP port for raw and WebSocket clients [default: 9000, or the next free one up to 9019]
    #[arg(long)]
    pub port: Option<u16>,
    /// Address to listen on
    #[arg(long, default_value = "0.0.0.0")]
    pub bind: IpAddr,
    /// Also serve WebSocket only on this port
    #[arg(long, value_name = "PORT")]
    pub ws_port: Option<u16>,
    /// Also accept a UDP client
    #[arg(long)]
    pub udp: bool,
    /// UDP port [default: the TCP port]
    #[arg(long, value_name = "PORT")]
    pub udp_port: Option<u16>,
    /// Do not advertise over mDNS
    #[arg(long)]
    pub no_mdns: bool,

    /// Accept clients from any address without asking
    #[arg(long)]
    pub allow_all: bool,
    /// Accept clients from this network without asking (e.g. 192.168.1.0/24); repeatable
    #[arg(long, value_name = "CIDR")]
    pub allow_from: Vec<Cidr>,
    /// Browser origins allowed from this machine (glob); repeatable [default: localhost, 127.0.0.1, vscode-webview, https://uclog.unitcircle.ca]
    #[arg(long, value_name = "GLOB")]
    pub allow_origin: Vec<String>,
    /// TLS certificate chain (PEM) for wss:// and TLS-TCP
    #[arg(long, value_name = "FILE", requires = "tls_key")]
    pub tls_cert: Option<PathBuf>,
    /// TLS private key (PEM)
    #[arg(long, value_name = "FILE", requires = "tls_cert")]
    pub tls_key: Option<PathBuf>,

    /// The hub answers all device syscalls itself (stdout discarded, stdin EOF, files in --files-dir)
    #[arg(long)]
    pub syscalls: bool,
    /// Sandbox directory for --syscalls [default: the current directory]
    #[arg(long, value_name = "DIR")]
    pub files_dir: Option<PathBuf>,
    /// Forward non-time syscalls to the connected client
    #[arg(long, value_enum, default_value = "on")]
    pub client_syscalls: OnOff,

    /// Device frames kept for clients that connect later (0 = only the latest device info)
    #[arg(long, value_name = "BYTES", default_value_t = 1 << 20)]
    pub backlog: usize,
    /// What to do when a client cannot keep up
    #[arg(long, value_enum, default_value = "drop")]
    pub slow_client: SlowClient,
    /// End a client session after this long without bytes from it
    #[arg(long, value_name = "SECS", default_value_t = 5.0)]
    pub client_timeout: f64,
    /// Longest encoded frame accepted
    #[arg(long, value_name = "N", default_value_t = uclog_proto::MAX_FRAME)]
    pub max_frame: usize,

    /// Print {"port":…,"ws_port":…,"udp_port":…,"name":…,"device":…} on stdout once listening
    #[arg(long)]
    pub json_ready: bool,
    /// Exit when stdin reaches EOF (the parent process died)
    #[arg(long)]
    pub exit_on_stdin_eof: bool,
    /// Do not write the local registry file (tests)
    #[arg(long, hide = true)]
    pub no_registry: bool,
    /// Seconds between statistics lines (with -v)
    #[arg(long, value_name = "SECS", default_value_t = 10.0, hide = true)]
    pub stats_interval: f64,
}

impl HubArgs {
    pub fn client_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.client_timeout.max(0.5))
    }
}
