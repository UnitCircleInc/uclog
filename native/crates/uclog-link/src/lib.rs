//! Device and hub links (protocol.md §4) and discovery (plans README §5.1).
//!
//! Every link has the same shape: a [`Link`] with a sender for outgoing bytes and a receiver of
//! [`LinkEvent`]s. Writes are whole wire frames (or pulses); links do their own chunking (HID
//! reports, BLE MTU, STLINK split). Blocking backends (serial, HID) run on their own threads.

mod ble;
pub mod discover;
mod hid;
pub mod loopback;
mod net;
mod serial;
mod uri;

use std::fmt;

use tokio::sync::mpsc;

pub use discover::{Device, Kind};
pub use net::{is_too_big, set_dont_fragment, WS_BUSY};
pub use uri::{LinkSpec, UriError};

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("{0}")]
    Uri(#[from] UriError),
    #[error("serial: {0}")]
    Serial(#[from] serialport::Error),
    #[error("HID: {0}")]
    Hid(String),
    #[error("BLE: {0}")]
    Ble(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("WebSocket: {0}")]
    Ws(String),
    #[error("{0}")]
    NotFound(String),
}

/// What to do when Linux refuses access to a device (plan 07 task 9): one line each.
pub const LINUX_ACCESS_HINT: &[&str] = &[
    "Linux refused access to the device:",
    "- serial adapters (ST-LINK, FTDI): join the dialout group (uucp on Arch), then log in again:",
    "    sudo usermod -aG dialout $USER",
    "- ucLog USB devices (CDC, HID): install 70-uclog.rules from the release archive, then replug:",
    "    sudo cp 70-uclog.rules /etc/udev/rules.d/ && sudo udevadm control --reload && sudo udevadm trigger",
];

impl LinkError {
    /// On Linux, when the OS refused access to the device: how to get it (LINUX_ACCESS_HINT).
    pub fn access_hint(&self) -> Option<&'static [&'static str]> {
        (cfg!(target_os = "linux") && self.is_access_denied()).then_some(LINUX_ACCESS_HINT)
    }

    fn is_access_denied(&self) -> bool {
        match self {
            LinkError::Serial(e) => {
                e.kind == serialport::ErrorKind::Io(std::io::ErrorKind::PermissionDenied)
                    || e.description.contains("Permission denied")
            }
            LinkError::Io(e) => e.kind() == std::io::ErrorKind::PermissionDenied,
            // hidapi's message carries the OS error's text.
            LinkError::Hid(m) => m.contains("Permission denied"),
            _ => false,
        }
    }
}

/// Something that happened on a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    /// Bytes received (any chunking).
    Data(Vec<u8>),
    /// The link closed. The session decides whether to reopen.
    Closed(CloseReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseReason {
    /// The device went away (unplugged, BLE disconnect).
    Detached(String),
    /// A hub turned the connection away because another client is connected.
    HubBusy,
    /// The peer closed the connection.
    Eof,
    Error(String),
}

impl fmt::Display for CloseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CloseReason::Detached(s) => write!(f, "device detached: {s}"),
            CloseReason::HubBusy => write!(f, "hub busy (another client is connected)"),
            CloseReason::Eof => write!(f, "connection closed"),
            CloseReason::Error(s) => write!(f, "{s}"),
        }
    }
}

/// An open link.
pub struct Link {
    pub spec: LinkSpec,
    /// Human-readable description (device name, serial, address).
    pub description: String,
    pub tx: mpsc::Sender<Vec<u8>>,
    pub rx: mpsc::Receiver<LinkEvent>,
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link").field("spec", &self.spec).field("description", &self.description).finish()
    }
}

const CHANNEL: usize = 256;

/// (writer for outgoing bytes, its receiver, event sender, event receiver).
type Channels = (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>, mpsc::Sender<LinkEvent>, mpsc::Receiver<LinkEvent>);

fn channels() -> Channels {
    let (tx, txr) = mpsc::channel(CHANNEL);
    let (ev, evr) = mpsc::channel(CHANNEL);
    (tx, txr, ev, evr)
}

/// Open a link. `Auto` must be resolved with [`discover::resolve_auto`] first.
pub async fn open(spec: &LinkSpec) -> Result<Link, LinkError> {
    match spec {
        LinkSpec::Serial { path, baud } => serial::open(spec.clone(), path, *baud),
        LinkSpec::Hid { id } => hid::open(spec.clone(), id).await,
        LinkSpec::Ble { id } => ble::open(spec.clone(), id).await,
        LinkSpec::Udp { addr } => net::open_udp(spec.clone(), addr).await,
        LinkSpec::Tcp { addr } => net::open_tcp(spec.clone(), addr).await,
        LinkSpec::Ws { url } => net::open_ws(spec.clone(), url).await,
        LinkSpec::Loopback { opts } => loopback::open(spec.clone(), opts),
        LinkSpec::Auto => Err(LinkError::NotFound("`auto` must be resolved to a device first".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_denied() {
        let serial = LinkError::Serial(serialport::Error::new(
            serialport::ErrorKind::Io(std::io::ErrorKind::PermissionDenied),
            "Permission denied",
        ));
        let hid = LinkError::Hid("Failed to open a device with path '/dev/hidraw3': Permission denied".into());
        assert!(serial.is_access_denied() && hid.is_access_denied());
        assert!(!LinkError::NotFound("no device".into()).is_access_denied());
        assert!(!LinkError::Hid("device disconnected".into()).is_access_denied());
        // The hint is for Linux only: elsewhere the OS asks (macOS) or it does not apply.
        assert_eq!(serial.access_hint().is_some(), cfg!(target_os = "linux"));
    }
}
