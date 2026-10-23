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
