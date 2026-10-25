//! The device-frame ring: backlog for clients that connect later, and the live queue of the
//! connected client (hub plan §5.1, D6, D23).
//!
//! Frames are stored COBS-encoded without delimiters. Each has a sequence number; the client
//! writer keeps a cursor (the next sequence number to send). The device side never waits for the
//! client: when the ring is full the oldest frames are evicted, and a cursor that falls behind
//! skips ahead (the skipped frames are counted).

use std::collections::VecDeque;
use std::sync::Arc;

/// Capacity used for the live queue when the backlog is disabled (`--backlog 0`).
pub const LIVE_ONLY_BYTES: usize = 64 * 1024;

#[derive(Debug)]
struct Entry {
    seq: u64,
    frame: Arc<[u8]>,
    info: bool,
}

#[derive(Debug)]
pub struct Backlog {
    replay: bool,
    max_bytes: usize,
    bytes: usize,
    entries: VecDeque<Entry>,
    next_seq: u64,
    /// The port-62 frame in effect at the oldest entry (the last one evicted).
    info_at_oldest: Option<Arc<[u8]>>,
    latest_info: Option<Arc<[u8]>>,
}

/// What a client gets on accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// Device info to send first (if any).
    pub info: Option<Arc<[u8]>>,
    /// Cursor: the first frame to send after it.
    pub cursor: u64,
}

/// Frames read from a cursor.
#[derive(Debug, Default)]
pub struct Read {
    pub frames: Vec<Arc<[u8]>>,
    /// Frames the cursor had fallen behind on (evicted before they were sent).
    pub skipped: u64,
    pub cursor: u64,
}

impl Backlog {
    /// `bytes == 0` disables the backlog: clients get only the latest device info, then live data.
    pub fn new(bytes: usize) -> Self {
        Backlog {
            replay: bytes > 0,
            max_bytes: if bytes > 0 { bytes } else { LIVE_ONLY_BYTES },
            bytes: 0,
            entries: VecDeque::new(),
            next_seq: 0,
            info_at_oldest: None,
            latest_info: None,
        }
    }

    /// Store one device frame (not a pulse, not port 63). `info`: it is a port-62 frame.
    pub fn push(&mut self, frame: &[u8], info: bool) -> u64 {
        let frame: Arc<[u8]> = frame.into();
        if info {
            self.latest_info = Some(frame.clone());
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.bytes += frame.len();
        self.entries.push_back(Entry { seq, frame, info });
        while self.bytes > self.max_bytes && self.entries.len() > 1 {
            let e = self.entries.pop_front().unwrap();
            self.bytes -= e.frame.len();
            if e.info {
                self.info_at_oldest = Some(e.frame);
            }
        }
        seq
    }

    fn oldest_seq(&self) -> u64 {
        self.entries.front().map(|e| e.seq).unwrap_or(self.next_seq)
    }

    /// Where a newly accepted client starts.
    pub fn start(&self) -> Start {
        if self.replay {
            // An info frame at the oldest entry is sent anyway, as the first backlog frame.
            let oldest_is_info = self.entries.front().is_some_and(|e| e.info);
            let info = if oldest_is_info { None } else { self.info_at_oldest.clone() };
            Start { info, cursor: self.oldest_seq() }
        } else {
            Start { info: self.latest_info.clone(), cursor: self.next_seq }
        }
    }

    /// Frames from `cursor`, up to `max_bytes` on the wire: each frame plus its two `00`
    /// delimiters (at least one frame if any is available).
    pub fn read(&self, cursor: u64, max_bytes: usize) -> Read {
        let oldest = self.oldest_seq();
        let (skipped, cursor) = if cursor < oldest { (oldest - cursor, oldest) } else { (0, cursor) };
        let mut r = Read { frames: vec![], skipped, cursor };
        let start = (cursor - oldest) as usize;
        let mut n = 0;
        for e in self.entries.iter().skip(start) {
            if !r.frames.is_empty() && n + e.frame.len() + 2 > max_bytes {
                break;
            }
            n += e.frame.len() + 2;
            r.frames.push(e.frame.clone());
            r.cursor = e.seq + 1;
        }
        r
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn latest_info(&self) -> Option<Arc<[u8]>> {
        self.latest_info.clone()
    }

    /// (frames, bytes) currently held.
    pub fn len(&self) -> (usize, usize) {
        (self.entries.len(), self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(tag: u8, len: usize) -> Vec<u8> {
        vec![tag; len]
    }

    #[test]
    fn replay_from_oldest_with_info_at_oldest() {
        let mut b = Backlog::new(30);
        b.push(&f(1, 10), true); // info A
        b.push(&f(2, 10), false);
        b.push(&f(3, 10), false);
        assert_eq!(b.start(), Start { info: None, cursor: 0 });
        b.push(&f(4, 10), true); // info B; evicts info A
        let s = b.start();
        assert_eq!(s.info.as_deref(), Some(&f(1, 10)[..]));
        assert_eq!(s.cursor, 1);
        let r = b.read(s.cursor, 1000);
        assert_eq!(r.frames.len(), 3);
        assert_eq!(r.frames[2].as_ref(), &f(4, 10)[..]);
        assert_eq!((r.skipped, r.cursor), (0, 4));
        // Evict the frames before info B: B is the oldest entry, so no extra info first.
        b.push(&f(5, 10), false);
        b.push(&f(6, 10), false);
        assert_eq!(b.start(), Start { info: None, cursor: 3 });
        // Evict B too: B is sent first.
        b.push(&f(7, 10), false);
        assert_eq!(b.start().info.as_deref(), Some(&f(4, 10)[..]));
        assert_eq!(b.latest_info().as_deref(), Some(&f(4, 10)[..]));
    }

    #[test]
    fn slow_cursor_skips_and_counts() {
        let mut b = Backlog::new(25);
        for i in 0..10 {
            b.push(&f(i, 10), false);
        }
        let r = b.read(0, 1000);
        assert_eq!(r.skipped, 8);
        assert_eq!(r.frames.len(), 2);
        assert_eq!(r.cursor, 10);
        assert!(b.read(10, 1000).frames.is_empty());
    }

    #[test]
    fn read_respects_max_bytes() {
        let mut b = Backlog::new(1000);
        for i in 0..5 {
            b.push(&f(i, 100), false);
        }
        let r = b.read(0, 250);
        assert_eq!((r.frames.len(), r.cursor), (2, 2));
        let r = b.read(0, 10); // always at least one
        assert_eq!(r.frames.len(), 1);
    }

    #[test]
    fn disabled_backlog_replays_latest_info_only() {
        let mut b = Backlog::new(0);
        b.push(&f(1, 10), true);
        b.push(&f(2, 10), false);
        let s = b.start();
        assert_eq!(s.info.as_deref(), Some(&f(1, 10)[..]));
        assert_eq!(s.cursor, 2);
        b.push(&f(3, 10), false);
        assert_eq!(b.read(s.cursor, 1000).frames.len(), 1);
    }
}
