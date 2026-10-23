//! Host clock sources for the time ops (protocol.md §5.5.3).

use uclog_proto::semihost::TimeSample;

pub trait TimeSource: Send + Sync {
    fn sample(&self) -> TimeSample;
}

/// The OS monotonic counter (QPC on Windows, `CLOCK_MONOTONIC` in ns elsewhere) and the
/// wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl SystemClock {
    pub fn new() -> Self {
        SystemClock
    }
}

fn wall() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

#[cfg(unix)]
fn mono() -> (u64, u64) {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: valid pointer to a timespec; CLOCK_MONOTONIC is always available.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64, 1_000_000_000)
}

#[cfg(windows)]
fn mono() -> (u64, u64) {
    use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    let (mut c, mut f) = (0i64, 0i64);
    // SAFETY: valid pointers; these calls cannot fail on Windows XP and later.
    unsafe {
        QueryPerformanceCounter(&mut c);
        QueryPerformanceFrequency(&mut f);
    }
    (c as u64, f as u64)
}

#[cfg(not(any(unix, windows)))]
fn mono() -> (u64, u64) {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    (START.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64, 1_000_000_000)
}

impl TimeSource for SystemClock {
    fn sample(&self) -> TimeSample {
        let (ticks, freq) = mono();
        TimeSample { ticks, freq, wall: wall() }
    }
}
