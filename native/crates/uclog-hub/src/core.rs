//! Hub core: device task, client sessions, shared state (hub plan §3, §5, §6).

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, watch, Notify};
use tokio::time::{interval, sleep, sleep_until, Instant};
use tracing::{debug, info, warn};
use uclog_link::{CloseReason, Link, LinkError, LinkEvent};
use uclog_proto::frame::{peek, wire_port, Peek};
use uclog_proto::scanner::{FrameScanner, ScanEvent};
use uclog_proto::timing::{LIVENESS_TIMEOUT, PULSE_INTERVAL, WAKE_GAP};
use uclog_proto::{cobs, ports, MAX_WIRE};

use crate::backlog::Backlog;
use crate::config::SlowClient;
use crate::syscalls::{Route, Router};

/// Opens (and reopens) the device link.
pub type Opener = Box<dyn Fn() -> Pin<Box<dyn Future<Output = Result<Link, LinkError>> + Send>> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    /// Link not open (yet, or again).
    Detached,
    /// Link open, no bytes yet.
    Waiting,
    Alive,
    /// No bytes for more than 2 s.
    Silent,
}

#[derive(Debug, Default)]
pub struct Counters {
    pub dev_frames: AtomicU64,
    pub dev_bytes: AtomicU64,
    pub dev_framing_errors: AtomicU64,
    pub to_dev_frames: AtomicU64,
    pub to_dev_dropped: AtomicU64,
    pub client_frames_out: AtomicU64,
    pub client_dropped: AtomicU64,
    pub client_framing_errors: AtomicU64,
    pub sessions: AtomicU64,
}

/// Who holds the client slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    Free,
    Pending(SocketAddr),
    Active(SocketAddr),
}

pub struct Shared {
    pub backlog: Mutex<Backlog>,
    /// Woken when the backlog grows, a device pulse arrives, or the device state changes.
    pub notify: Notify,
    pub device_pulse: AtomicBool,
    pub router: Mutex<Router>,
    pub slot: Mutex<Slot>,
    /// `true` while a client is active (mDNS / registry `busy`).
    pub busy: watch::Sender<bool>,
    pub state: watch::Sender<DeviceState>,
    /// Wire bytes for the device (client frames, syscall replies).
    pub to_device: mpsc::Sender<Vec<u8>>,
    /// Encoded port-63 requests for the active client.
    client_syscall: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    pub counters: Counters,
    pub max_frame: usize,
    pub slow_client: SlowClient,
    pub client_timeout: Duration,
}

impl Shared {
    pub fn new(
        backlog: usize,
        router: Router,
        max_frame: usize,
        slow_client: SlowClient,
        client_timeout: Duration,
    ) -> (Arc<Self>, mpsc::Receiver<Vec<u8>>) {
        let (to_device, rx) = mpsc::channel(256);
        let s = Shared {
            backlog: Mutex::new(Backlog::new(backlog)),
            notify: Notify::new(),
            device_pulse: AtomicBool::new(false),
            router: Mutex::new(router),
            slot: Mutex::new(Slot::Free),
            busy: watch::channel(false).0,
            state: watch::channel(DeviceState::Detached).0,
            to_device,
            client_syscall: Mutex::new(None),
            counters: Counters::default(),
            max_frame,
            slow_client,
            client_timeout,
        };
        (Arc::new(s), rx)
    }

    /// Claim the slot for a new connection; `None` = busy.
    pub fn claim(self: &Arc<Self>, peer: SocketAddr) -> Option<SlotGuard> {
        let mut s = self.slot.lock().unwrap();
        if *s != Slot::Free {
            return None;
        }
        *s = Slot::Pending(peer);
        Some(SlotGuard { shared: self.clone(), peer })
    }

    fn set_state(&self, st: DeviceState) {
        if *self.state.borrow() != st {
            info!("device {}", format!("{st:?}").to_lowercase());
            self.state.send_replace(st);
            self.notify.notify_one();
        }
    }
}

/// Holds the client slot; dropping it frees the slot (and answers an outstanding syscall).
pub struct SlotGuard {
    shared: Arc<Shared>,
    peer: SocketAddr,
}

impl SlotGuard {
    pub fn activate(&self) {
        *self.shared.slot.lock().unwrap() = Slot::Active(self.peer);
        self.shared.busy.send_replace(true);
    }
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        let was_active = matches!(*self.shared.slot.lock().unwrap(), Slot::Active(_));
        *self.shared.slot.lock().unwrap() = Slot::Free;
        *self.shared.client_syscall.lock().unwrap() = None;
        if let Some(r) = self.shared.router.lock().unwrap().client_gone() {
            info!("client left with a syscall outstanding: replying EIO to the device");
            let _ = self.shared.to_device.try_send(wire_port(ports::SEMIHOST, &r));
        }
        if was_active {
            self.shared.busy.send_replace(false);
            info!("client {} disconnected", self.peer);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Device task

/// Runs forever: opens the device, reads and writes it, reopens it when it goes away.
pub async fn device_task(shared: Arc<Shared>, opener: Opener, mut to_device: mpsc::Receiver<Vec<u8>>) {
    let mut failures = 0u32;
    loop {
        shared.set_state(DeviceState::Detached);
        let mut link = match opener().await {
            Ok(l) => {
                failures = 0;
                info!("device open: {}", l.description);
                l
            }
            Err(e) => {
                if failures == 0 {
                    warn!("device: {e} (retrying)");
                }
                failures += 1;
                // Drop what clients send while there is no device.
                while let Ok(_d) = to_device.try_recv() {
                    shared.counters.to_dev_dropped.fetch_add(1, Relaxed);
                }
                let backoff = if matches!(e, LinkError::Ble(_)) { (failures as u64 * 500).min(5000) } else { 500 };
                sleep(Duration::from_millis(backoff)).await;
                continue;
            }
        };
        shared.set_state(DeviceState::Waiting);
        let reason = run_link(&shared, &mut link, &mut to_device).await;
        warn!("device closed: {reason}");
        sleep(Duration::from_millis(500)).await;
    }
}

async fn run_link(shared: &Arc<Shared>, link: &mut Link, to_device: &mut mpsc::Receiver<Vec<u8>>) -> CloseReason {
    let mut scanner = FrameScanner::new(shared.max_frame);
    // Wake the device (its UART may be powered down) and give it time (protocol.md §2.3).
    if link.tx.send(vec![0]).await.is_err() {
        return CloseReason::Eof;
    }
    let mut last_tx = Instant::now();
    let ready_at = last_tx + WAKE_GAP;
    let mut last_rx: Option<Instant> = None;
    let mut tick = interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            ev = link.rx.recv() => match ev {
                Some(LinkEvent::Data(d)) => {
                    last_rx = Some(Instant::now());
                    shared.set_state(DeviceState::Alive);
                    shared.counters.dev_bytes.fetch_add(d.len() as u64, Relaxed);
                    let mut replies = vec![];
                    scanner.feed_with(&d, |e| match e {
                        ScanEvent::Frame(f) => {
                            if let Some(r) = on_device_frame(shared, &f) { replies.push(r); }
                        }
                        ScanEvent::Oversize(n) => {
                            shared.counters.dev_framing_errors.fetch_add(1, Relaxed);
                            debug!("device frame over the size limit: {n} bytes discarded");
                        }
                    });
                    for r in replies {
                        if link.tx.send(r).await.is_err() { return CloseReason::Eof; }
                        last_tx = Instant::now();
                    }
                }
                Some(LinkEvent::Closed(r)) => return r,
                None => return CloseReason::Eof,
            },
            d = to_device.recv() => {
                let Some(d) = d else { return CloseReason::Eof };
                if Instant::now() < ready_at { sleep_until(ready_at).await; }
                shared.counters.to_dev_frames.fetch_add(1, Relaxed);
                if link.tx.send(d).await.is_err() { return CloseReason::Eof; }
                last_tx = Instant::now();
            }
            _ = tick.tick() => {
                if last_tx.elapsed() >= PULSE_INTERVAL {
                    if link.tx.send(vec![0]).await.is_err() { return CloseReason::Eof; }
                    last_tx = Instant::now();
                }
                if last_rx.is_some_and(|t| t.elapsed() > LIVENESS_TIMEOUT) {
                    shared.set_state(DeviceState::Silent);
                }
            }
        }
    }
}

/// One encoded device frame. Returns a reply for the device (syscalls answered by the hub).
fn on_device_frame(shared: &Arc<Shared>, f: &[u8]) -> Option<Vec<u8>> {
    match peek(f) {
        Peek::Pulse => {
            shared.device_pulse.store(true, Relaxed);
            shared.notify.notify_one();
            None
        }
        Peek::Port(ports::SEMIHOST) => {
            let Ok(payload) = cobs::decode(f) else {
                shared.counters.dev_framing_errors.fetch_add(1, Relaxed);
                return None;
            };
            let req = &payload[1..];
            let client = shared.client_syscall.lock().unwrap().clone();
            let route = shared.router.lock().unwrap().from_device(req, client.is_some());
            match route {
                Route::Reply(r) => Some(wire_port(ports::SEMIHOST, &r)),
                Route::None => None,
                Route::Forward => {
                    if client.is_some_and(|c| c.try_send(f.to_vec()).is_ok()) {
                        None
                    } else {
                        // The client queue is gone or full: answer as if it had left.
                        shared.router.lock().unwrap().client_gone().map(|r| wire_port(ports::SEMIHOST, &r))
                    }
                }
            }
        }
        p => {
            shared.counters.dev_frames.fetch_add(1, Relaxed);
            shared.backlog.lock().unwrap().push(f, p == Peek::Port(ports::DEVICE_INFO));
            shared.notify.notify_one();
            None
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Client session

/// An accepted client connection, transport-independent.
pub struct ClientConn {
    pub peer: SocketAddr,
    pub kind: &'static str,
    /// Bytes from the client; closed = disconnected.
    pub rx: mpsc::Receiver<Vec<u8>>,
    /// Bytes to the client. Each message holds whole frames, at most `max_chunk` bytes.
    pub tx: mpsc::Sender<Vec<u8>>,
    pub max_chunk: usize,
}

fn push_frame(out: &mut Vec<u8>, f: &[u8]) {
    out.push(0);
    out.extend_from_slice(f);
    out.push(0);
}

/// Serve one client until it leaves. The slot must already be active.
pub async fn client_session(shared: Arc<Shared>, guard: SlotGuard, mut c: ClientConn) {
    shared.counters.sessions.fetch_add(1, Relaxed);
    info!("client {} connected ({})", c.peer, c.kind);
    let (sys_tx, mut sys_rx) = mpsc::channel::<Vec<u8>>(16);
    *shared.client_syscall.lock().unwrap() = Some(sys_tx);
    let start = shared.backlog.lock().unwrap().start();
    let mut cursor = start.cursor;
    let mut first = vec![];
    if let Some(info) = &start.info {
        push_frame(&mut first, info);
    }
    let chunk = c.max_chunk.max(MAX_WIRE);
    let mut scanner = FrameScanner::new(shared.max_frame);
    let mut last_rx = Instant::now();
    let mut last_tx = Instant::now();
    let mut tick = interval(Duration::from_millis(100));
    let mut dropped = 0u64;
    // A forwarded request goes out after the backlog frames that preceded it (device order).
    let mut sys_pending: Vec<Vec<u8>> = vec![];
    let end: String = 'session: loop {
        // Device → client: device info first, then the backlog / live frames.
        let mut out = std::mem::take(&mut first);
        loop {
            let r = shared.backlog.lock().unwrap().read(cursor, chunk - out.len().min(chunk));
            if r.skipped > 0 {
                if shared.slow_client == SlowClient::Disconnect {
                    break 'session format!("too slow: {} frames behind", r.skipped);
                }
                dropped += r.skipped;
                shared.counters.client_dropped.fetch_add(r.skipped, Relaxed);
            }
            cursor = r.cursor;
            let n = r.frames.len();
            for f in r.frames {
                if out.len() + f.len() + 2 > chunk && !out.is_empty() {
                    break; // cannot happen with read()'s limit, but keep datagrams whole
                }
                push_frame(&mut out, &f);
            }
            shared.counters.client_frames_out.fetch_add(n as u64, Relaxed);
            if out.is_empty() {
                break;
            }
            if c.tx.send(std::mem::take(&mut out)).await.is_err() {
                break 'session "connection closed".into();
            }
            last_tx = Instant::now();
            if n == 0 {
                break;
            }
        }
        for f in sys_pending.drain(..) {
            let mut o = vec![];
            push_frame(&mut o, &f);
            if c.tx.send(o).await.is_err() {
                break 'session "connection closed".into();
            }
            last_tx = Instant::now();
        }
        if shared.device_pulse.swap(false, Relaxed) {
            if c.tx.send(vec![0]).await.is_err() {
                break 'session "connection closed".into();
            }
            last_tx = Instant::now();
        }
        tokio::select! {
            _ = shared.notify.notified() => {}
            s = sys_rx.recv() => if let Some(f) = s { sys_pending.push(f) },
            d = c.rx.recv() => {
                let Some(d) = d else { break 'session "disconnected".into() };
                last_rx = Instant::now();
                let mut frames = vec![];
                scanner.feed_with(&d, |e| match e {
                    ScanEvent::Frame(f) => frames.push(f),
                    ScanEvent::Oversize(_) => { shared.counters.client_framing_errors.fetch_add(1, Relaxed); }
                });
                for f in frames {
                    match peek(&f) {
                        Peek::Pulse => continue, // the hub pulses the device itself
                        Peek::Port(ports::SEMIHOST) => shared.router.lock().unwrap().from_client(),
                        _ => {}
                    }
                    let mut w = Vec::with_capacity(f.len() + 2);
                    push_frame(&mut w, &f);
                    if *shared.state.borrow() == DeviceState::Detached || shared.to_device.try_send(w).is_err() {
                        shared.counters.to_dev_dropped.fetch_add(1, Relaxed);
                    }
                }
            }
            _ = tick.tick() => {
                if last_rx.elapsed() > shared.client_timeout {
                    break 'session format!("no data for {:?}", shared.client_timeout);
                }
                // Pulse the client while the device is alive (protocol.md §2.3); silence tells the
                // client the device is not responding.
                if last_tx.elapsed() >= PULSE_INTERVAL && *shared.state.borrow() == DeviceState::Alive {
                    if c.tx.send(vec![0]).await.is_err() { break 'session "connection closed".into(); }
                    last_tx = Instant::now();
                }
            }
        }
    };
    if dropped > 0 {
        warn!("client {}: {dropped} frames dropped (slow client)", c.peer);
    }
    info!("client {} session ended: {end}", c.peer);
    drop(guard);
}
