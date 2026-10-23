//! ucLog wire protocol v1 (`spec/protocol.md`).
//!
//! Pure protocol code: no I/O, no threads, no async runtime, so it can later be compiled to
//! WebAssembly (plans README §4.1).

pub mod cbor;
pub mod cobs;
pub mod crc32c;
pub mod frame;
pub mod fw;
pub mod link;
pub mod scanner;
pub mod semihost;

pub use frame::{mux_decode, mux_encode, peek, wire, Mux, Peek};
pub use scanner::{FrameScanner, ScanEvent};

/// Well-known ports (protocol.md §5.1).
pub mod ports {
    pub const COMMANDS: u8 = 0;
    pub const PRISM: u8 = 1;
    pub const FW_LEGACY: u8 = 3;
    pub const FW: u8 = 59;
    pub const PLOT: u8 = 60;
    pub const TERMINAL: u8 = 61;
    pub const DEVICE_INFO: u8 = 62;
    pub const SEMIHOST: u8 = 63;
}

/// Timing constants (protocol.md §2.3).
pub mod timing {
    use std::time::Duration;

    /// Send something at least this often while connected.
    pub const PULSE_INTERVAL: Duration = Duration::from_millis(500);
    /// No bytes for longer than this: the peer is not responding.
    pub const LIVENESS_TIMEOUT: Duration = Duration::from_secs(2);
    /// Pause after a wake pulse when TX was idle for longer than [`LIVENESS_TIMEOUT`].
    pub const WAKE_GAP: Duration = Duration::from_millis(50);
}

/// Maximum encoded frame size every implementation must support (protocol.md §2.2).
pub const MAX_FRAME: usize = 4096;
