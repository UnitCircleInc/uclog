//! FW update message encoding for ports 59 and 3 (protocol.md §5.4).

use crate::cbor::{self, Value};
use crate::{cobs, crc32c::crc32c, ports, scanner::FrameScanner, ScanEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FwPort {
    /// Port 59: plain CBOR.
    New,
    /// Port 3: `00 cobs(cbor ‖ crc32c_le) 00`.
    Legacy,
}

impl FwPort {
    pub fn port(self) -> u8 {
        match self {
            FwPort::New => ports::FW,
            FwPort::Legacy => ports::FW_LEGACY,
        }
    }
}

/// Port payload for a message.
pub fn encode(port: FwPort, msg: &Value) -> Vec<u8> {
    let c = cbor::to_vec(msg);
    match port {
        FwPort::New => c,
        FwPort::Legacy => {
            let mut body = c;
            body.extend_from_slice(&crc32c(&body).to_le_bytes());
            let mut v = vec![0];
            v.extend(cobs::encode(&body));
            v.push(0);
            v
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FwDecodeError {
    #[error("no inner frame")]
    NoFrame,
    #[error("invalid inner COBS: {0}")]
    Cobs(#[from] cobs::CobsError),
    #[error("inner frame too short")]
    Short,
    #[error("CRC mismatch")]
    Crc,
    #[error("invalid CBOR: {0}")]
    Cbor(String),
}

/// Decode a port payload into a CBOR message.
pub fn decode(port: FwPort, data: &[u8]) -> Result<Value, FwDecodeError> {
    let body = match port {
        FwPort::New => data.to_vec(),
        FwPort::Legacy => {
            let mut sc = FrameScanner::default();
            let ev = sc.feed(data);
            let Some(ScanEvent::Frame(f)) = ev.into_iter().next() else {
                return Err(FwDecodeError::NoFrame);
            };
            let inner = cobs::decode(&f)?;
            if inner.len() < 4 {
                return Err(FwDecodeError::Short);
            }
            let (b, c) = inner.split_at(inner.len() - 4);
            if crc32c(b) != u32::from_le_bytes(c.try_into().unwrap()) {
                return Err(FwDecodeError::Crc);
            }
            b.to_vec()
        }
    };
    cbor::from_slice(&body).map_err(|e| FwDecodeError::Cbor(e.to_string()))
}

pub fn set_time(timestamp: i64) -> Value {
    cbor::map([("cmd", Value::Text("set-time".into())), ("timestamp", timestamp.into())])
}

pub fn fw_start() -> Value {
    cbor::map([("cmd", Value::Text("fw-start".into()))])
}

pub fn fw_block(data: &[u8]) -> Value {
    cbor::map([("cmd", Value::Text("fw-block".into())), ("data", Value::Bytes(data.to_vec()))])
}

pub fn fw_schedule(timestamp: i64) -> Value {
    cbor::map([("cmd", Value::Text("fw-schedule".into())), ("timestamp", timestamp.into())])
}
