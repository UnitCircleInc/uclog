//! Frame header, two-byte peek and mux (protocol.md §3).

use crate::cobs;

pub const TYPE_LOG: u8 = 0;
pub const TYPE_MEM: u8 = 1;
pub const TYPE_RES: u8 = 2;
pub const TYPE_PORT: u8 = 3;
pub const TARGET_DIGIT_SHIFT: u32 = 20;

/// Result of the two-byte peek on an encoded frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peek {
    Pulse,
    /// Log frame of the given type (0, 1 or 2).
    Log(u8),
    Port(u8),
}

/// Classify an encoded (still COBS) frame by reading at most `encoded[0..2]`.
pub fn peek(encoded: &[u8]) -> Peek {
    if encoded.is_empty() || encoded == [0x01] {
        return Peek::Pulse;
    }
    let b0 = if encoded[0] == 0x01 { 0 } else { encoded.get(1).copied().unwrap_or(0) };
    match b0 & 3 {
        TYPE_PORT => Peek::Port(b0 >> 2),
        t => Peek::Log(t),
    }
}

/// A decoded payload split by its header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mux<'a> {
    Pulse,
    Port {
        port: u8,
        data: &'a [u8],
    },
    Log {
        ty: u8,
        target: u8,
        addr: u32,
        args: &'a [u8],
    },
    /// Type 0-2 payload shorter than 4 bytes.
    ShortLog(&'a [u8]),
}

pub fn mux_decode(payload: &[u8]) -> Mux<'_> {
    let Some(&b0) = payload.first() else { return Mux::Pulse };
    if b0 & 3 == TYPE_PORT {
        return Mux::Port { port: b0 >> 2, data: &payload[1..] };
    }
    if payload.len() < 4 {
        return Mux::ShortLog(payload);
    }
    let addr = u32::from_le_bytes(payload[..4].try_into().unwrap());
    Mux::Log { ty: b0 & 3, target: ((addr >> TARGET_DIGIT_SHIFT) & 0xF) as u8, addr, args: &payload[4..] }
}

/// Payload for a port frame.
pub fn mux_encode(port: u8, data: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(data.len() + 1);
    v.push((port << 2) | TYPE_PORT);
    v.extend_from_slice(data);
    v
}

/// Complete wire encoding of one payload: `00 cobs(payload) 00`.
pub fn wire(payload: &[u8]) -> Vec<u8> {
    let enc = cobs::encode(payload);
    let mut v = Vec::with_capacity(enc.len() + 2);
    v.push(0);
    v.extend_from_slice(&enc);
    v.push(0);
    v
}

/// Wire bytes for a port frame.
pub fn wire_port(port: u8, data: &[u8]) -> Vec<u8> {
    wire(&mux_encode(port, data))
}

/// The lone-`00` pulse.
pub const PULSE: &[u8] = &[0];
