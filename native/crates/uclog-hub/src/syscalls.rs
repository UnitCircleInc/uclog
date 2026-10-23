//! Port-63 routing (hub plan §6, protocol.md §5.5.5).
//!
//! | Situation | Time ops | Other ops |
//! |---|---|---|
//! | `--syscalls` | hub | hub (full handler, stdio discarded) |
//! | client connected | hub | client; the hub tracks the outstanding request |
//! | client connected, `--client-syscalls off` | hub | hub default: `ENOSYS` |
//! | no client | hub | hub default: `ENOSYS` |
//!
//! When a client leaves with a request outstanding, the hub replies `EIO` on its behalf.

use std::path::PathBuf;

use uclog_proto::semihost::{self as sh, errno, Request};
use uclog_semihost::{Config, Handler, Mode, Outcome, Stdio, TimeSource};

/// What to do with a device request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Send this reply to the device.
    Reply(Vec<u8>),
    /// Forward the request to the client.
    Forward,
    /// Nothing to send (a WRITE that wants no reply).
    None,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub hub: u64,
    pub forwarded: u64,
    pub client_replies: u64,
    pub eio_on_disconnect: u64,
}

pub struct Router {
    /// `--syscalls`: the hub answers everything.
    full: Option<Handler>,
    /// Time ops always; everything else ENOSYS.
    default: Handler,
    client_syscalls: bool,
    outstanding: Option<u8>,
    pub stats: Stats,
}

fn needs_reply(req: &[u8]) -> bool {
    !matches!(sh::parse_request(req), Ok(Request::Write { reply: false, .. }))
}

impl Router {
    /// `files_dir`: `Some` enables `--syscalls` with that sandbox.
    pub fn new(files_dir: Option<PathBuf>, client_syscalls: bool) -> Self {
        let full =
            files_dir.map(|d| Handler::new(Config { mode: Mode::Full, stdio: Stdio::Discard, files_dir: Some(d) }));
        let default = Handler::new(Config { mode: Mode::TimeOnly, stdio: Stdio::Discard, files_dir: None });
        Router { full, default, client_syscalls, outstanding: None, stats: Stats::default() }
    }

    /// With a given clock (tests).
    pub fn with_clock(files_dir: Option<PathBuf>, client_syscalls: bool, clock: Box<dyn TimeSource>) -> Self {
        let mut r = Router::new(None, client_syscalls);
        r.default = Handler::with_clock(Config { mode: Mode::TimeOnly, stdio: Stdio::Discard, files_dir: None }, clock);
        r.full =
            files_dir.map(|d| Handler::new(Config { mode: Mode::Full, stdio: Stdio::Discard, files_dir: Some(d) }));
        r
    }

    fn answer(h: &mut Handler, req: &[u8]) -> Route {
        match h.handle(req) {
            Outcome::Done { reply: Some(r), .. } => Route::Reply(r),
            Outcome::Done { reply: None, .. } => Route::None,
            // Stdio::Discard never waits for stdin; be safe anyway.
            Outcome::WaitStdin { .. } => Route::Reply(sh::read_reply(&[])),
        }
    }

    /// A request from the device (port-63 data).
    pub fn from_device(&mut self, req: &[u8], client_connected: bool) -> Route {
        let op = req.first().copied().unwrap_or(0xFF);
        if sh::is_time_op(op) {
            self.stats.hub += 1;
            return Self::answer(&mut self.default, req);
        }
        if let Some(full) = &mut self.full {
            self.stats.hub += 1;
            return Self::answer(full, req);
        }
        if client_connected && self.client_syscalls {
            self.stats.forwarded += 1;
            if needs_reply(req) {
                self.outstanding = Some(op);
            }
            return Route::Forward;
        }
        self.stats.hub += 1;
        Self::answer(&mut self.default, req)
    }

    /// A port-63 frame from the client (a reply): it is forwarded to the device.
    pub fn from_client(&mut self) {
        self.stats.client_replies += 1;
        self.outstanding = None;
    }

    /// The client left: the reply the device is still waiting for, if any.
    pub fn client_gone(&mut self) -> Option<Vec<u8>> {
        let op = self.outstanding.take()?;
        self.stats.eio_on_disconnect += 1;
        Some(sh::error_reply(op, errno::EIO))
    }

    pub fn outstanding(&self) -> Option<u8> {
        self.outstanding
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uclog_proto::semihost::op;
    use uclog_proto::semihost::TimeSample;

    struct Fixed;
    impl TimeSource for Fixed {
        fn sample(&self) -> TimeSample {
            TimeSample { ticks: 5_000_000_000, freq: 1_000_000_000, wall: 1_700_000_000.25 }
        }
    }

    fn router(files: Option<PathBuf>, client_syscalls: bool) -> Router {
        Router::with_clock(files, client_syscalls, Box::new(Fixed))
    }

    fn write(fid: u32, data: &[u8]) -> Vec<u8> {
        let mut r = vec![op::WRITE];
        r.extend_from_slice(&fid.to_le_bytes());
        r.extend_from_slice(data);
        r
    }

    #[test]
    fn time_ops_always_answered_by_the_hub() {
        for (client, cs) in [(false, true), (true, true), (true, false)] {
            let mut r = router(None, cs);
            assert_eq!(r.from_device(&[op::SYS_TIME], client), Route::Reply(1_700_000_000i64.to_le_bytes().to_vec()));
            assert_eq!(
                r.from_device(&[op::SYS_TICKFREQ], client),
                Route::Reply(1_000_000_000u64.to_le_bytes().to_vec())
            );
            assert_eq!(r.outstanding(), None);
        }
    }

    #[test]
    fn no_client_gets_enosys() {
        let mut r = router(None, true);
        let req = [op::ISATTY, 1, 0, 0, 0];
        assert_eq!(r.from_device(&req, false), Route::Reply(sh::error_reply(op::ISATTY, errno::ENOSYS)));
        // A WRITE that wants no reply gets none.
        assert_eq!(r.from_device(&write(1 | sh::WRITE_NO_REPLY, b"x"), false), Route::None);
    }

    #[test]
    fn client_connected_forwards_and_tracks() {
        let mut r = router(None, true);
        assert_eq!(r.from_device(&write(1, b"hi"), true), Route::Forward);
        assert_eq!(r.outstanding(), Some(op::WRITE));
        r.from_client();
        assert_eq!(r.outstanding(), None);
        assert_eq!(r.client_gone(), None);
        // No-reply WRITE is forwarded but not outstanding.
        assert_eq!(r.from_device(&write(1 | sh::WRITE_NO_REPLY, b"x"), true), Route::Forward);
        assert_eq!(r.outstanding(), None);
        // Disconnect with a READ outstanding: EIO.
        assert_eq!(r.from_device(&[op::READ, 0, 0, 0, 0, 64, 0, 0, 0], true), Route::Forward);
        assert_eq!(r.client_gone(), Some(sh::error_reply(op::READ, errno::EIO)));
        assert_eq!(r.client_gone(), None);
    }

    #[test]
    fn client_syscalls_off_uses_default() {
        let mut r = router(None, false);
        let req = [op::CLOSE, 3, 0, 0, 0];
        assert_eq!(r.from_device(&req, true), Route::Reply(sh::error_reply(op::CLOSE, errno::ENOSYS)));
    }

    #[test]
    fn hub_syscalls_full_handler() {
        let d = tempfile::tempdir().unwrap();
        let mut r = router(Some(d.path().into()), true);
        // stdout discarded with the full length; not forwarded even with a client.
        assert_eq!(r.from_device(&write(1, b"hello"), true), Route::Reply(sh::reply(5, 0)));
        // stdin is EOF.
        assert_eq!(r.from_device(&[op::READ, 0, 0, 0, 0, 64, 0, 0, 0], true), Route::Reply(sh::read_reply(&[])));
        // Files work in the sandbox.
        let mut open = vec![op::OPEN];
        open.extend_from_slice(&(0x601i32).to_le_bytes()); // WRONLY|CREAT|TRUNC
        open.extend_from_slice(b"x.txt");
        let Route::Reply(rep) = r.from_device(&open, true) else { panic!() };
        assert_eq!(&rep[4..8], &0i32.to_le_bytes());
        assert!(d.path().join("x.txt").exists());
        // Escapes are refused.
        let mut esc = vec![op::OPEN];
        esc.extend_from_slice(&0i32.to_le_bytes());
        esc.extend_from_slice(b"../x");
        assert_eq!(r.from_device(&esc, true), Route::Reply(sh::error_reply(op::OPEN, errno::EACCES)));
    }
}
