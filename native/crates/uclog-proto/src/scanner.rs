//! Frame scanner (protocol.md §2.2).

/// Event produced by [`FrameScanner::feed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEvent {
    /// COBS-encoded bytes between two delimiters (non-empty).
    Frame(Vec<u8>),
    /// A run longer than the maximum frame size was discarded (number of bytes).
    Oversize(usize),
}

/// Splits a byte stream on `0x00`.
///
/// Bytes before the first `0x00` are dropped. Empty runs are ignored (they are the gap between
/// frames, and lone-`00` pulses); use "any byte received" for liveness.
#[derive(Debug)]
pub struct FrameScanner {
    max_frame: usize,
    buf: Vec<u8>,
    discarding: usize,
    synced: bool,
}

impl Default for FrameScanner {
    fn default() -> Self {
        Self::new(crate::MAX_FRAME)
    }
}

impl FrameScanner {
    pub fn new(max_frame: usize) -> Self {
        Self { max_frame, buf: Vec::new(), discarding: 0, synced: false }
    }

    /// Forget any partial frame and wait for the next delimiter (e.g. after a link reopen).
    pub fn reset(&mut self) {
        self.buf.clear();
        self.discarding = 0;
        self.synced = false;
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<ScanEvent> {
        let mut events = Vec::new();
        self.feed_with(data, |e| events.push(e));
        events
    }

    pub fn feed_with(&mut self, data: &[u8], mut emit: impl FnMut(ScanEvent)) {
        for &b in data {
            if b == 0 {
                if self.discarding > 0 {
                    emit(ScanEvent::Oversize(self.discarding));
                    self.discarding = 0;
                } else if !self.buf.is_empty() {
                    emit(ScanEvent::Frame(std::mem::take(&mut self.buf)));
                }
                self.buf.clear();
                self.synced = true;
            } else if !self.synced {
                continue;
            } else if self.discarding > 0 {
                self.discarding += 1;
            } else {
                self.buf.push(b);
                if self.buf.len() > self.max_frame {
                    self.discarding = self.buf.len();
                    self.buf.clear();
                }
            }
        }
    }
}
