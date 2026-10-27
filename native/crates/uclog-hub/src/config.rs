//! Hub command line (hub plan §8), shared by `uclog hub` and `uclog-hub`.

use std::ffi::OsString;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
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
    /// Options from a TOML file, keyed by their long names (port = 9001, allow-from = ["10.0.0.0/8"],
    /// no-mdns = true); the command line overrides it (repeatable options add to it)
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
}

/// `args` with `--config FILE` (or `--config=FILE`) replaced by the file's options, so clap reads
/// them where the option was: put it before the other options, and with `args_override_self`
/// those win. Keys are the long option names (`_` may stand for `-`); a value is a string, number
/// or boolean (true: the flag; false: nothing), or an array of them for a repeatable option.
pub fn expand_config(args: Vec<OsString>) -> anyhow::Result<Vec<OsString>> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let file = match a.to_str() {
            Some("--config") => Some(it.next().ok_or_else(|| anyhow::anyhow!("--config needs a FILE"))?),
            Some(s) if s.starts_with("--config=") => Some(OsString::from(&s["--config=".len()..])),
            _ => None,
        };
        match file {
            Some(f) => out.extend(config_args(Path::new(&f))?),
            None => out.push(a),
        }
    }
    Ok(out)
}

fn config_args(path: &Path) -> anyhow::Result<Vec<OsString>> {
    let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("--config {}: {e}", path.display()))?;
    let table: toml::Table = text.parse().map_err(|e| anyhow::anyhow!("--config {}: {e}", path.display()))?;
    let mut out = vec![];
    for (key, value) in table {
        let opt = format!("--{}", key.replace('_', "-"));
        if opt == "--config" {
            anyhow::bail!("--config {}: a config file cannot name another", path.display());
        }
        let values = match value {
            toml::Value::Array(v) => v,
            v => vec![v],
        };
        for v in values {
            match v {
                toml::Value::Boolean(true) => out.push(OsString::from(&opt)),
                toml::Value::Boolean(false) => {}
                toml::Value::String(s) => out.push(OsString::from(format!("{opt}={s}"))),
                toml::Value::Integer(n) => out.push(OsString::from(format!("{opt}={n}"))),
                toml::Value::Float(x) => out.push(OsString::from(format!("{opt}={x}"))),
                v => anyhow::bail!(
                    "--config {}: {key}: {} is not a string, number or boolean",
                    path.display(),
                    v.type_str()
                ),
            }
        }
    }
    Ok(out)
}

impl HubArgs {
    pub fn client_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.client_timeout.max(0.5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    #[command(args_override_self = true)]
    struct Cli {
        #[command(flatten)]
        hub: HubArgs,
    }

    fn parse(file: &str, args: &[&str]) -> anyhow::Result<HubArgs> {
        let dir = tempfile::tempdir()?;
        let f = dir.path().join("hub.toml");
        std::fs::write(&f, file)?;
        let mut a: Vec<OsString> = vec!["uclog-hub".into()];
        a.extend(args.iter().map(|s| OsString::from(s.replace("FILE", f.to_str().unwrap()))));
        Ok(Cli::try_parse_from(expand_config(a)?)?.hub)
    }

    #[test]
    fn the_file_sets_options() {
        let h = parse(
            "device = 'loopback:'\nport = 9005\nno_mdns = true\nudp = false\nallow-from = ['10.0.0.0/8', '192.168.1.0/24']\nclient-timeout = 2.5\nslow-client = 'disconnect'\n",
            &["--config", "FILE"],
        )
        .unwrap();
        assert_eq!(h.device.to_string(), "loopback:");
        assert_eq!(h.port, Some(9005));
        assert!(h.no_mdns && !h.udp);
        assert_eq!(h.allow_from.len(), 2);
        assert_eq!(h.client_timeout, 2.5);
        assert_eq!(h.slow_client, SlowClient::Disconnect);
    }

    #[test]
    fn the_command_line_wins() {
        let h = parse(
            "port = 9005\nallow-from = ['10.0.0.0/8']\n",
            &["--config=FILE", "--port", "9010", "--allow-from", "172.16.0.0/12"],
        )
        .unwrap();
        assert_eq!(h.port, Some(9010));
        assert_eq!(h.allow_from.len(), 2); // repeatable: added to the file's
    }

    #[test]
    fn bad_files_are_errors() {
        assert!(parse("port = 'x'\n", &["--config", "FILE"]).is_err());
        assert!(parse("no-such-option = 1\n", &["--config", "FILE"]).is_err());
        assert!(parse("[table]\nx = 1\n", &["--config", "FILE"]).is_err());
        assert!(parse("config = 'other.toml'\n", &["--config", "FILE"]).is_err());
        assert!(parse("port = 9005\n", &["--config", "/no/such/file.toml"]).is_err());
    }
}
