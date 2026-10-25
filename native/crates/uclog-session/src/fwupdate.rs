//! FW update client (protocol.md §5.4): port 59 first, port 3 fallback.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use uclog_proto::cbor::{self, Value};
use uclog_proto::fw::{self, FwPort};

use crate::SessionHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FwProgress {
    /// Probing for a port.
    Probing,
    /// Using this port (59 or 3).
    Port(u8),
    /// `fw-start` sent; the device is erasing.
    Starting,
    Block {
        done: usize,
        total: usize,
    },
    Scheduling,
    Done,
}

#[derive(Debug, thiserror::Error)]
pub enum FwError {
    #[error("device does not support FW update (no reply on port 59 or port 3)")]
    Unsupported,
    #[error("no reply to {0} within {1:?}")]
    Timeout(&'static str, Duration),
    #[error("device reported an error for {cmd}: {msg}")]
    Device { cmd: &'static str, msg: String },
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("session closed")]
    Closed,
    #[error("image is empty")]
    Empty,
    #[error(
        "device block-size {block} is too large: a fw-block frame on port {port} would carry {len} bytes, over the {max}-byte port limit (protocol.md §2.4, §5.4)"
    )]
    BlockTooLarge { block: usize, port: u8, len: usize, max: usize },
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

struct Fw<'a> {
    h: &'a SessionHandle,
    rx: mpsc::Receiver<(u8, Vec<u8>)>,
}

impl Fw<'_> {
    /// Send `msg` on `port`; wait for the reply whose `rsp` matches, ignoring events.
    async fn call(&mut self, port: FwPort, cmd: &'static str, msg: Value, timeout: Duration) -> Result<Value, FwError> {
        self.h.send(port.port(), fw::encode(port, &msg)).await.map_err(|_| FwError::Closed)?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let (p, data) = match tokio::time::timeout_at(deadline, self.rx.recv()).await {
                Err(_) => return Err(FwError::Timeout(cmd, timeout)),
                Ok(None) => return Err(FwError::Closed),
                Ok(Some(x)) => x,
            };
            if p != port.port() {
                continue;
            }
            let Ok(v) = fw::decode(port, &data) else { continue };
            if cbor::get(&v, "event").is_some() {
                continue;
            }
            if cbor::get_text(&v, "rsp") != Some(cmd) {
                continue;
            }
            if let Some(e) = cbor::get(&v, "error") {
                let msg = e.as_text().map(str::to_string).unwrap_or_else(|| cbor::display(e));
                return Err(FwError::Device { cmd, msg });
            }
            return Ok(v);
        }
    }
}

/// Run a FW update with `image` (the signed `.bin`). Progress is reported through `progress`.
pub async fn run(h: &SessionHandle, image: &[u8], progress: impl Fn(FwProgress)) -> Result<u8, FwError> {
    if image.is_empty() {
        return Err(FwError::Empty);
    }
    let mut fw = Fw { h, rx: h.subscribe_fw().await.map_err(|_| FwError::Closed)? };
    progress(FwProgress::Probing);
    let port = match fw.call(FwPort::New, "set-time", fw::set_time(now()), Duration::from_secs(1)).await {
        Ok(_) => FwPort::New,
        Err(FwError::Timeout(..)) => {
            // Also a probe: 1 s (protocol.md §5.4).
            match fw.call(FwPort::Legacy, "set-time", fw::set_time(now()), Duration::from_secs(1)).await {
                Ok(_) => FwPort::Legacy,
                Err(FwError::Timeout(..)) => return Err(FwError::Unsupported),
                Err(e) => return Err(e),
            }
        }
        Err(e) => return Err(e),
    };
    progress(FwProgress::Port(port.port()));
    progress(FwProgress::Starting);
    let r = fw.call(port, "fw-start", fw::fw_start(), Duration::from_secs(60)).await?;
    let block = cbor::get_int(&r, "block-size")
        .filter(|b| *b > 0)
        .ok_or(FwError::Device { cmd: "fw-start", msg: "reply has no block-size".into() })? as usize;
    // A full block must fit in one frame; otherwise the device never sees it and the update
    // would only time out. Worst case: no zero bytes, for the most port-3 COBS overhead.
    let len = fw::encode(port, &fw::fw_block(&vec![0xFF; block.min(image.len())])).len();
    if len > uclog_proto::MAX_PORT_DATA {
        return Err(FwError::BlockTooLarge { block, port: port.port(), len, max: uclog_proto::MAX_PORT_DATA });
    }
    let total = image.len().div_ceil(block);
    for (i, chunk) in image.chunks(block).enumerate() {
        fw.call(port, "fw-block", fw::fw_block(chunk), Duration::from_secs(5)).await?;
        progress(FwProgress::Block { done: i + 1, total });
    }
    progress(FwProgress::Scheduling);
    fw.call(port, "fw-schedule", fw::fw_schedule(now() + 5), Duration::from_secs(5)).await?;
    progress(FwProgress::Done);
    Ok(port.port())
}
