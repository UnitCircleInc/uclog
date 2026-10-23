//! COBS (protocol.md §2.1).

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CobsError {
    #[error("empty input")]
    Empty,
    #[error("zero byte at offset {0}")]
    Zero(usize),
    #[error("code 0x{code:02x} at offset {at} runs past the end ({len} bytes)")]
    Overrun { code: u8, at: usize, len: usize },
}

/// Encode `data`. A final full 0xFF block that ends exactly at the end of the input is not
/// followed by a 0x01 code byte.
pub fn encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 254 + 2);
    let mut code_at = 0;
    out.push(0);
    let mut code: u8 = 1;
    for (i, &b) in data.iter().enumerate() {
        if b == 0 {
            out[code_at] = code;
            code_at = out.len();
            out.push(0);
            code = 1;
        } else {
            out.push(b);
            code += 1;
            if code == 0xFF {
                out[code_at] = code;
                if i + 1 == data.len() {
                    return out;
                }
                code_at = out.len();
                out.push(0);
                code = 1;
            }
        }
    }
    out[code_at] = code;
    out
}

/// Decode one encoded frame (without delimiters).
pub fn decode(data: &[u8]) -> Result<Vec<u8>, CobsError> {
    if data.is_empty() {
        return Err(CobsError::Empty);
    }
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let code = data[i];
        if code == 0 {
            return Err(CobsError::Zero(i));
        }
        let end = i + code as usize;
        if end > data.len() {
            return Err(CobsError::Overrun { code, at: i, len: data.len() });
        }
        let seg = &data[i + 1..end];
        if let Some(z) = seg.iter().position(|&b| b == 0) {
            return Err(CobsError::Zero(i + 1 + z));
        }
        out.extend_from_slice(seg);
        i = end;
        if code < 0xFF || i == data.len() {
            out.push(0);
        }
    }
    out.pop();
    Ok(out)
}
