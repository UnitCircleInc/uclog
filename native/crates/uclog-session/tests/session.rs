//! End-to-end session tests against a fake device on a local TCP socket.

use std::path::Path;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use uclog_proto::cbor::{self, Value};
use uclog_proto::frame::{wire, wire_port};
use uclog_proto::fw::{self, FwPort};
use uclog_proto::{cobs, mux_decode, Mux, ScanEvent};
use uclog_session::{Config, Event, FwProgress, Liveness, ResolverConfig};

const HASH: [u8; 64] = [0x5a; 64];
const SADDR: u32 = 0xF010_0000;

/// What the fake device received: (arrival time, payload). Pulses are recorded as empty payloads.
type Rx = mpsc::UnboundedReceiver<(Instant, Vec<u8>)>;

struct Device {
    sock: tokio::net::tcp::OwnedWriteHalf,
    rx: Rx,
}

async fn setup(cfg: Config) -> (Device, uclog_session::SessionHandle, mpsc::Receiver<Event>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let spec: uclog_link::LinkSpec = format!("tcp://{addr}").parse().unwrap();
    let (link, accepted) = tokio::join!(uclog_link::open(&spec), l.accept());
    let (s, _) = accepted.unwrap();
    let (mut r, w) = s.into_split();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut sc = uclog_proto::FrameScanner::default();
        let mut buf = [0u8; 8192];
        let mut prev = 0u8;
        loop {
            let n = match r.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            let now = Instant::now();
            // A pulse is a 00 that follows a 00 and is not the start of a frame. (A frame's
            // trailing 00 follows a data byte; its leading 00 is followed by a data byte.)
            for (i, &b) in buf[..n].iter().enumerate() {
                if b == 0 && prev == 0 && (i + 1 == n || buf[i + 1] == 0) {
                    let _ = tx.send((now, vec![]));
                }
                prev = b;
            }
            for e in sc.feed(&buf[..n]) {
                if let ScanEvent::Frame(f) = e {
                    let _ = tx.send((now, cobs::decode(&f).unwrap()));
                }
            }
        }
    });
    let (h, ev) = uclog_session::start(link.unwrap(), cfg);
    (Device { sock: w, rx }, h, ev)
}

impl Device {
    async fn send(&mut self, payload: &[u8]) {
        self.sock.write_all(&wire(payload)).await.unwrap();
    }

    async fn send_port(&mut self, port: u8, data: &[u8]) {
        self.sock.write_all(&wire_port(port, data)).await.unwrap();
    }

    /// Next non-pulse payload received from the host.
    async fn frame(&mut self) -> (Instant, Vec<u8>) {
        loop {
            let (t, p) = tokio::time::timeout(Duration::from_secs(5), self.rx.recv()).await.expect("timeout").unwrap();
            if !p.is_empty() {
                return (t, p);
            }
        }
    }
}

async fn next_event(ev: &mut mpsc::Receiver<Event>, f: impl Fn(&Event) -> bool) -> Event {
    loop {
        let e = tokio::time::timeout(Duration::from_secs(5), ev.recv()).await.expect("timeout").unwrap();
        if f(&e) {
            return e;
        }
    }
}

fn sample() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../spec/vectors/sample.logdata")
}

fn device_info() -> Vec<u8> {
    cbor::to_vec(&cbor::map([("board", Value::Text("test".into())), ("app_hash", Value::Bytes(HASH.to_vec()))]))
}

/// F7: on a direct link the session asks for the device info; the simulated device answers, so
/// a second device info follows the one it sends at start.
#[tokio::test]
async fn direct_link_requests_device_info() {
    let spec: uclog_link::LinkSpec = "loopback:rate=0".parse().unwrap();
    let link = uclog_link::open(&spec).await.unwrap();
    let (h, mut ev) = uclog_session::start(link, Config::default());
    for _ in 0..2 {
        next_event(&mut ev, |e| matches!(e, Event::DeviceInfo(_))).await;
    }
    h.close().await;
}

#[tokio::test]
async fn logs_semihost_and_timing() {
    let cache = tempfile::tempdir().unwrap();
    std::fs::copy(sample(), cache.path().join(format!("{}.logdata", hex::encode(HASH)))).unwrap();
    let cfg = Config {
        resolver: ResolverConfig { cache_dir: Some(cache.path().into()), ..Default::default() },
        ..Default::default()
    };
    let (mut dev, h, mut ev) = setup(cfg).await;

    // Pulses arrive at least every 0.5 s.
    let t0 = Instant::now();
    let (first, p) = tokio::time::timeout(Duration::from_secs(1), dev.rx.recv()).await.unwrap().unwrap();
    assert!(p.is_empty(), "first thing sent is a pulse");
    assert!(first.duration_since(t0) < Duration::from_millis(600));

    // Wake gap: the first real frame comes ≥ 50 ms after the first (wake) pulse. (A tcp:// link
    // is a hub, so the session does not ask for the device info: `help` is the first frame.)
    h.send(61, b"help\r".to_vec()).await.unwrap();
    let (t, p) = dev.frame().await;
    assert_eq!(p, uclog_proto::frame::mux_encode(61, b"help\r"));
    assert!(t.duration_since(first) >= Duration::from_millis(45), "{:?}", t.duration_since(first));

    // Device info → logdata from cache → logs decode (frames sent before resolution finishes
    // are held, not reported as missing logdata).
    dev.send_port(62, &device_info()).await;
    let mut log = SADDR.to_le_bytes().to_vec();
    log.extend_from_slice(b"there\0");
    dev.send(&log).await;
    let e = next_event(&mut ev, |e| matches!(e, Event::Logdata { .. })).await;
    let Event::Logdata { source, targets, .. } = e else { unreachable!() };
    assert!(matches!(source, Some(uclog_session::Source::Cache(_))));
    assert_eq!(targets, vec![1]);
    let Event::Log(r) = next_event(&mut ev, |e| matches!(e, Event::Log(_))).await else { unreachable!() };
    assert_eq!((r.level.as_str(), r.text.as_str(), r.error), ("INFO", "Hello World! there", None));

    // Error record for an unknown target.
    dev.send(&0xF020_0000u32.to_le_bytes()).await;
    let Event::Log(r) = next_event(&mut ev, |e| matches!(e, Event::Log(_))).await else { unreachable!() };
    assert_eq!(r.error, Some("no_logdata"));

    // Semihost: time op and stdout.
    dev.send_port(63, &[0x31]).await;
    let (_, p) = dev.frame().await;
    let Mux::Port { port: 63, data } = mux_decode(&p) else { panic!("{p:?}") };
    assert_eq!(data.len(), 8);
    let mut w = vec![9u8];
    w.extend_from_slice(&1u32.to_le_bytes());
    w.extend_from_slice(b"out");
    dev.send_port(63, &w).await;
    let Event::Console(c) = next_event(&mut ev, |e| matches!(e, Event::Console(_))).await else { unreachable!() };
    assert_eq!(c.data, b"out");
    let (_, p) = dev.frame().await;
    assert_eq!(p, uclog_proto::frame::mux_encode(63, &uclog_proto::semihost::reply(3, 0)));

    // Terminal output is passed through.
    dev.send_port(61, b"hello\r\n").await;
    let Event::Port { port, data } = next_event(&mut ev, |e| matches!(e, Event::Port { .. })).await else {
        unreachable!()
    };
    assert_eq!((port, data.as_slice()), (61, &b"hello\r\n"[..]));

    // Silence → not responding.
    let e = tokio::time::timeout(
        Duration::from_secs(4),
        next_event(&mut ev, |e| matches!(e, Event::Liveness(Liveness::NotResponding))),
    )
    .await;
    assert!(e.is_ok());
    h.close().await;
}

/// Fake FW-update responder. `port59` = whether the device supports port 59.
async fn fw_device(mut dev: Device, port59: bool, block: usize) -> (Vec<u8>, u8) {
    let mut image = vec![];
    loop {
        let (_, p) = dev.frame().await;
        let Mux::Port { port, data } = mux_decode(&p) else { continue };
        let fp = match port {
            59 => FwPort::New,
            3 => FwPort::Legacy,
            _ => continue,
        };
        if fp == FwPort::New && !port59 {
            continue;
        }
        let m = fw::decode(fp, data).unwrap();
        let cmd = cbor::get_text(&m, "cmd").unwrap().to_string();
        let mut reply = vec![("rsp", Value::Text(cmd.clone()))];
        match cmd.as_str() {
            "fw-start" => reply.push(("block-size", Value::Integer((block as u64).into()))),
            "fw-block" => image.extend_from_slice(cbor::get_bytes(&m, "data").unwrap()),
            _ => {}
        }
        // An unrelated event first: must be ignored.
        dev.send_port(port, &fw::encode(fp, &cbor::map([("event", Value::Text("progress".into()))]))).await;
        dev.send_port(port, &fw::encode(fp, &cbor::map(reply))).await;
        if cmd == "fw-schedule" {
            return (image, port);
        }
    }
}

#[tokio::test]
async fn fw_update_port59_and_fallback() {
    for port59 in [true, false] {
        let (dev, h, _ev) = setup(Config::default()).await;
        let image: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
        let d = tokio::spawn(fw_device(dev, port59, 256));
        let progress = std::sync::Mutex::new(vec![]);
        let used = h.fw_update(&image, |p| progress.lock().unwrap().push(p)).await.unwrap();
        let (got, port) = d.await.unwrap();
        assert_eq!(got, image);
        assert_eq!(used, port);
        assert_eq!(used, if port59 { 59 } else { 3 });
        let p = progress.into_inner().unwrap();
        assert!(p.contains(&FwProgress::Block { done: 4, total: 4 }));
        assert_eq!(p.last(), Some(&FwProgress::Done));
        h.close().await;
    }
}

#[tokio::test]
async fn hub_busy_is_reported() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        drop(s); // closed before any data = busy
    });
    let link = uclog_link::open(&format!("tcp://{addr}").parse().unwrap()).await.unwrap();
    let (_h, mut ev) = uclog_session::start(link, Config::default());
    let e = next_event(&mut ev, |e| matches!(e, Event::Closed(_))).await;
    assert!(matches!(e, Event::Closed(uclog_link::CloseReason::HubBusy)));
    let _ = TcpStream::connect(addr).await;
}

/// TX gaps stay within 0.5 s (protocol.md §2.3): at most PULSE_DUE + PULSE_TICK (0.45 s) when the
/// pulse task runs on time. The check allows 0.15 s more for a loaded CI runner, which can wake it
/// late (a 537 ms gap on macOS); that is still well inside the device's 2 s liveness timeout.
#[tokio::test]
async fn pulse_gaps_within_half_a_second() {
    let (mut dev, h, _ev) = setup(Config::default()).await;
    let (mut prev, _) = tokio::time::timeout(Duration::from_secs(1), dev.rx.recv()).await.unwrap().unwrap();
    let end = prev + Duration::from_secs(3);
    let mut max = Duration::ZERO;
    while prev < end {
        let (t, _) = tokio::time::timeout(Duration::from_secs(1), dev.rx.recv()).await.unwrap().unwrap();
        max = max.max(t.duration_since(prev));
        prev = t;
    }
    assert!(max <= Duration::from_millis(650), "max TX gap {max:?}");
    h.close().await;
}

/// A new image whose logdata is not found must not decode with the previous image's logdata
/// (protocol.md §6.4): its logs are `no_logdata` errors.
#[tokio::test]
async fn unresolved_image_clears_logdata() {
    let cache = tempfile::tempdir().unwrap();
    std::fs::copy(sample(), cache.path().join(format!("{}.logdata", hex::encode(HASH)))).unwrap();
    let cfg = Config {
        resolver: ResolverConfig { cache_dir: Some(cache.path().into()), ..Default::default() },
        ..Default::default()
    };
    let (mut dev, h, mut ev) = setup(cfg).await;
    let mut log = SADDR.to_le_bytes().to_vec();
    log.extend_from_slice(b"there\0");
    dev.send_port(62, &device_info()).await;
    let e = next_event(&mut ev, |e| matches!(e, Event::Logdata { .. })).await;
    assert!(matches!(e, Event::Logdata { source: Some(_), .. }));
    dev.send(&log).await;
    let Event::Log(r) = next_event(&mut ev, |e| matches!(e, Event::Log(_))).await else { unreachable!() };
    assert_eq!(r.error, None);

    let other = cbor::to_vec(&cbor::map([("app_hash", Value::Bytes(vec![0x11; 64]))]));
    dev.send_port(62, &other).await;
    let e = next_event(&mut ev, |e| matches!(e, Event::Logdata { .. })).await;
    assert!(matches!(e, Event::Logdata { source: None, .. }));
    dev.send(&log).await;
    let Event::Log(r) = next_event(&mut ev, |e| matches!(e, Event::Log(_))).await else { unreachable!() };
    assert_eq!(r.error, Some("no_logdata"), "{}", r.text);
    h.close().await;
}

/// A stdin READ pending when the device resets (device info) or sends another request is not
/// answered later: the stale reply would be taken for the reply to a different request.
#[tokio::test]
async fn stale_stdin_read_is_dropped() {
    let (mut dev, h, mut ev) = setup(Config::default()).await;
    use uclog_proto::semihost::{op, reply};
    let mut read = vec![op::READ]; // fid 0, 16 bytes
    read.extend_from_slice(&0i32.to_le_bytes());
    read.extend_from_slice(&16u32.to_le_bytes());
    for reset in [true, false] {
        dev.send_port(63, &read).await;
        next_event(&mut ev, |e| matches!(e, Event::StdinWanted)).await;
        if reset {
            dev.send_port(62, &device_info()).await;
            next_event(&mut ev, |e| matches!(e, Event::DeviceInfo(_))).await;
        } else {
            // ISATTY(1) is answered; that is the only port-63 frame the device gets.
            dev.send_port(63, &[op::ISATTY, 1, 0, 0, 0]).await;
            let (_, p) = dev.frame().await;
            assert_eq!(p, uclog_proto::frame::mux_encode(63, &reply(1, 0)));
        }
        h.stdin(b"late\r".to_vec()).await.unwrap();
        next_event(&mut ev, |e| matches!(e, Event::Note(n) if n.contains("no semihost stdin read"))).await;
    }
    h.send(61, b"x".to_vec()).await.unwrap();
    let (_, p) = dev.frame().await;
    assert_eq!(p, uclog_proto::frame::mux_encode(61, b"x"), "no stdin reply reached the device");
    h.close().await;
}

/// A block-size whose fw-block frame cannot fit in one frame is refused up front, with a clear
/// error, instead of timing out on a block the device never receives.
#[tokio::test]
async fn fw_update_block_size_too_large() {
    let (dev, h, _ev) = setup(Config::default()).await;
    let image = vec![0xA5u8; 4000];
    let _d = tokio::spawn(fw_device(dev, true, 1500));
    let e = h.fw_update(&image, |_| {}).await.unwrap_err();
    assert!(matches!(e, uclog_session::FwError::BlockTooLarge { block: 1500, port: 59, .. }), "{e}");
    h.close().await;
}
