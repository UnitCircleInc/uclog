//! Hub integration tests against the loopback device (hub plan §10). No hardware needed.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::{timeout, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use uclog_hub::Running;
use uclog_proto::frame::{mux_decode, peek, wire_port, Mux, Peek};
use uclog_proto::scanner::{FrameScanner, ScanEvent};
use uclog_proto::{cobs, ports};

#[derive(Parser)]
struct T {
    #[command(flatten)]
    h: uclog_hub::HubArgs,
}

async fn hub(extra: &[&str]) -> Running {
    let mut args = vec!["t", "--port", "0", "--bind", "127.0.0.1", "--no-mdns", "--no-registry"];
    args.extend_from_slice(extra);
    if !extra.contains(&"-d") {
        args.extend_from_slice(&["-d", "loopback:rate=20"]);
    }
    uclog_hub::start(T::parse_from(args).h, 0).await.unwrap()
}

/// Decoded payloads from a byte stream; pulses are counted, not returned.
#[derive(Default)]
struct Rx {
    sc: FrameScanner,
    frames: Vec<Vec<u8>>,
    pulses: usize,
    bytes: usize,
}

impl Rx {
    fn feed(&mut self, d: &[u8]) {
        self.bytes += d.len();
        let mut fs = vec![];
        self.sc.feed_with(d, |e| {
            if let ScanEvent::Frame(f) = e {
                fs.push(f)
            }
        });
        for f in fs {
            if peek(&f) == Peek::Pulse {
                self.pulses += 1;
            } else {
                self.frames.push(cobs::decode(&f).unwrap());
            }
        }
    }

    fn port_text(&self, port: u8) -> String {
        self.frames
            .iter()
            .filter_map(|p| match mux_decode(p) {
                Mux::Port { port: q, data } if q == port => Some(String::from_utf8_lossy(data).into_owned()),
                _ => None,
            })
            .collect()
    }

    fn count_port(&self, port: u8) -> usize {
        self.frames.iter().filter(|p| matches!(mux_decode(p), Mux::Port { port: q, .. } if q == port)).count()
    }
}

/// A raw TCP client that pulses like a real one.
struct Raw {
    s: TcpStream,
    rx: Rx,
}

impl Raw {
    async fn connect(port: u16) -> Raw {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(&[0]).await.unwrap();
        Raw { s, rx: Rx::default() }
    }

    async fn send(&mut self, port: u8, data: &[u8]) {
        self.s.write_all(&wire_port(port, data)).await.unwrap();
    }

    /// Read (and pulse) for `d`, or until `until` holds. Returns false on EOF.
    async fn pump(&mut self, d: Duration, until: impl Fn(&Rx) -> bool) -> bool {
        let end = Instant::now() + d;
        let mut buf = vec![0u8; 65536];
        let mut last_pulse = Instant::now();
        while Instant::now() < end && !until(&self.rx) {
            if last_pulse.elapsed() > Duration::from_millis(400) {
                let _ = self.s.write_all(&[0]).await;
                last_pulse = Instant::now();
            }
            match timeout(Duration::from_millis(100), self.s.read(&mut buf)).await {
                Ok(Ok(0)) | Ok(Err(_)) => return false,
                Ok(Ok(n)) => self.rx.feed(&buf[..n]),
                Err(_) => {}
            }
        }
        true
    }
}

fn is_info(p: &[u8]) -> bool {
    matches!(mux_decode(p), Mux::Port { port: ports::DEVICE_INFO, .. })
}

#[tokio::test]
async fn backlog_replay_starts_with_device_info() {
    let h = hub(&[]).await;
    tokio::time::sleep(Duration::from_millis(600)).await; // the device logs while nobody is connected
    let mut c = Raw::connect(h.port).await;
    assert!(c.pump(Duration::from_secs(2), |r| r.frames.len() >= 10).await);
    assert!(is_info(&c.rx.frames[0]), "first frame is the device info");
    let logs = c.rx.frames.iter().filter(|p| matches!(mux_decode(p), Mux::Log { .. })).count();
    assert!(logs >= 8, "backlog and live logs: {logs}");
    h.shutdown().await;
}

#[tokio::test]
async fn second_client_is_busy() {
    let h = hub(&[]).await;
    let mut c1 = Raw::connect(h.port).await;
    assert!(c1.pump(Duration::from_secs(1), |r| !r.frames.is_empty()).await);
    // Raw TCP: closed without data.
    let mut c2 = TcpStream::connect(("127.0.0.1", h.port)).await.unwrap();
    c2.write_all(&[0]).await.unwrap();
    let mut b = [0u8; 16];
    assert_eq!(timeout(Duration::from_secs(2), c2.read(&mut b)).await.unwrap().unwrap_or(0), 0);
    // The uclog client link reports it as HubBusy.
    let spec = format!("tcp://127.0.0.1:{}", h.port).parse().unwrap();
    let mut l = uclog_link::open(&spec).await.unwrap();
    l.tx.send(vec![0]).await.unwrap();
    let ev = timeout(Duration::from_secs(2), l.rx.recv()).await.unwrap();
    assert_eq!(ev, Some(uclog_link::LinkEvent::Closed(uclog_link::CloseReason::HubBusy)));
    // WebSocket: close code 4001, also HubBusy for the client.
    let spec = format!("ws://127.0.0.1:{}", h.port).parse().unwrap();
    let mut l = uclog_link::open(&spec).await.unwrap();
    let ev = timeout(Duration::from_secs(2), l.rx.recv()).await.unwrap();
    assert_eq!(ev, Some(uclog_link::LinkEvent::Closed(uclog_link::CloseReason::HubBusy)));
    // After the first client leaves, the slot is free again.
    drop(c1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut c3 = Raw::connect(h.port).await;
    assert!(c3.pump(Duration::from_secs(1), |r| !r.frames.is_empty()).await);
    h.shutdown().await;
}

#[tokio::test]
async fn terminal_round_trip_and_pulses() {
    let h = hub(&["-d", "loopback:rate=0"]).await;
    let mut c = Raw::connect(h.port).await;
    c.send(61, b"echo hello hub\r").await;
    assert!(c.pump(Duration::from_secs(2), |r| r.port_text(61).contains("hello hub")).await);
    // With no logs, the hub still pulses the client (lone 00s) while the device is alive.
    let (b0, f0) = (c.rx.bytes, c.rx.frames.len());
    c.pump(Duration::from_millis(1600), |_| false).await;
    assert_eq!(c.rx.frames.len(), f0);
    assert!(c.rx.bytes >= b0 + 2, "pulse bytes {} -> {}", b0, c.rx.bytes);
    h.shutdown().await;
}

#[tokio::test]
async fn silent_device_means_silent_hub() {
    let h = hub(&["-d", "loopback:rate=0"]).await;
    let mut c = Raw::connect(h.port).await;
    c.send(61, b"quiet 4\r").await;
    assert!(c.pump(Duration::from_secs(2), |r| r.port_text(61).contains("quiet for")).await);
    let b0 = c.rx.bytes;
    // The device sends nothing for 4 s: after its 2 s liveness timeout the hub stops pulsing.
    c.pump(Duration::from_millis(3000), |_| false).await;
    let b1 = c.rx.bytes;
    c.pump(Duration::from_millis(800), |_| false).await;
    assert_eq!(c.rx.bytes, b1, "no bytes while the device is silent (got {} before)", b1 - b0);
    h.shutdown().await;
}

#[tokio::test]
async fn time_ops_answered_by_hub_and_eio_on_disconnect() {
    let h = hub(&["-d", "loopback:rate=0"]).await;
    let mut c1 = Raw::connect(h.port).await;
    c1.send(61, b"sh\r").await;
    // TIMEOFDAY is answered by the hub; the WRITE with a reply is forwarded to us.
    assert!(c1.pump(Duration::from_secs(2), |r| r.count_port(63) >= 1).await);
    assert!(c1.rx.port_text(61).contains("TIMEOFDAY sec=Some("), "{}", c1.rx.port_text(61));
    assert_eq!(c1.rx.count_port(63), 1, "only the WRITE reaches the client");
    // Leave without replying: the hub answers EIO, and the device reports it.
    drop(c1);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut c2 = Raw::connect(h.port).await;
    assert!(c2.pump(Duration::from_secs(2), |r| r.port_text(61).contains("WRITE r=")).await);
    assert!(c2.rx.port_text(61).contains("sh: WRITE r=Some(-1) errno=Some(5)"), "{}", c2.rx.port_text(61));
    // A client that replies is forwarded to the device.
    c2.send(61, b"read\r").await;
    assert!(c2.pump(Duration::from_secs(2), |r| r.count_port(63) >= 1).await);
    let mut reply = 3i32.to_le_bytes().to_vec();
    reply.extend_from_slice(&0i32.to_le_bytes());
    reply.extend_from_slice(b"ok\n");
    c2.send(63, &reply).await;
    assert!(c2.pump(Duration::from_secs(2), |r| r.port_text(61).contains("read: n=3")).await);
    h.shutdown().await;
}

#[tokio::test]
async fn hub_syscalls_mode_answers_everything() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().to_str().unwrap();
    let h = hub(&["-d", "loopback:rate=0", "--syscalls", "--files-dir", dir]).await;
    let mut c = Raw::connect(h.port).await;
    c.send(61, b"sh\r").await;
    assert!(c.pump(Duration::from_secs(2), |r| r.port_text(61).contains("WRITE r=")).await);
    assert!(c.rx.port_text(61).contains("WRITE r=Some(20) errno=Some(0)"), "{}", c.rx.port_text(61));
    assert_eq!(c.rx.count_port(63), 0, "nothing forwarded");
    h.shutdown().await;
}

#[tokio::test]
async fn client_timeout_frees_the_slot() {
    let h = hub(&["--client-timeout", "1"]).await;
    let mut c1 = TcpStream::connect(("127.0.0.1", h.port)).await.unwrap();
    c1.write_all(&[0]).await.unwrap(); // then silence
    let mut buf = vec![0u8; 65536];
    let t0 = Instant::now();
    loop {
        match timeout(Duration::from_secs(3), c1.read(&mut buf)).await.unwrap() {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
    let t = t0.elapsed();
    assert!(t >= Duration::from_millis(900) && t < Duration::from_millis(2500), "closed after {t:?}");
    let mut c2 = Raw::connect(h.port).await;
    assert!(c2.pump(Duration::from_secs(1), |r| !r.frames.is_empty()).await);
    h.shutdown().await;
}

#[tokio::test]
async fn slow_client_drops_and_counts() {
    let h = hub(&["-d", "loopback:rate=0", "--backlog", "4000"]).await;
    let mut c = Raw::connect(h.port).await;
    // Fill the hub's queue while we do not read: big frames, many of them.
    for _ in 0..40 {
        c.send(61, b"big\r").await;
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    c.pump(Duration::from_secs(3), |_| false).await;
    let got = c.rx.count_port(61);
    let dropped = h.shared.counters.client_dropped.load(std::sync::atomic::Ordering::Relaxed);
    assert!(got < 40, "some frames were dropped (got {got})");
    assert_eq!(got as u64 + dropped, 40, "received + dropped = sent (got {got}, dropped {dropped})");
    h.shutdown().await;
}

#[tokio::test]
async fn websocket_on_the_same_port() {
    let h = hub(&["-d", "loopback:rate=0"]).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}/uclog", h.port)).await.unwrap();
    ws.send(Message::binary(wire_port(61, b"echo via ws\r"))).await.unwrap();
    let mut rx = Rx::default();
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end && !rx.port_text(61).contains("via ws") {
        if let Ok(Some(Ok(Message::Binary(b)))) = timeout(Duration::from_millis(200), ws.next()).await {
            rx.feed(&b);
        }
    }
    assert!(is_info(&rx.frames[0]));
    assert!(rx.port_text(61).contains("via ws"));
    h.shutdown().await;
}

#[tokio::test]
async fn browser_origin_not_allowed_is_rejected() {
    let h = hub(&[]).await;
    let mut req = format!("ws://127.0.0.1:{}/", h.port).into_client_request().unwrap();
    req.headers_mut().insert("origin", "https://evil.example".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    // No console to ask: rejected with a close frame, no data.
    let m = timeout(Duration::from_secs(2), ws.next()).await.unwrap();
    assert!(matches!(m, Some(Ok(Message::Close(Some(_))))), "{m:?}");
    // An allowed origin works.
    let mut req = format!("ws://127.0.0.1:{}/", h.port).into_client_request().unwrap();
    req.headers_mut().insert("origin", "http://localhost:5173".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let m = timeout(Duration::from_secs(2), ws.next()).await.unwrap();
    assert!(matches!(m, Some(Ok(Message::Binary(_)))), "{m:?}");
    h.shutdown().await;
}

fn local_ip() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // no packet is sent
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

#[tokio::test]
async fn remote_peer_needs_approval() {
    let Some(ip) = local_ip() else { return };
    let h = {
        let args = ["t", "--port", "0", "--bind", "0.0.0.0", "--no-mdns", "--no-registry", "-d", "loopback:rate=20"];
        uclog_hub::start(T::parse_from(args).h, 0).await.unwrap()
    };
    // From a non-loopback address, with no console and no allow rule: rejected.
    let mut s = TcpStream::connect(SocketAddr::new(ip, h.port)).await.unwrap();
    s.write_all(&[0]).await.unwrap();
    let mut b = [0u8; 64];
    assert_eq!(timeout(Duration::from_secs(2), s.read(&mut b)).await.unwrap().unwrap_or(0), 0);
    h.shutdown().await;
    // With --allow-from it is accepted.
    let cidr = format!("{ip}/32");
    let args = [
        "t",
        "--port",
        "0",
        "--bind",
        "0.0.0.0",
        "--no-mdns",
        "--no-registry",
        "-d",
        "loopback:rate=20",
        "--allow-from",
        &cidr,
    ];
    let h = uclog_hub::start(T::parse_from(args).h, 0).await.unwrap();
    let mut s = TcpStream::connect(SocketAddr::new(ip, h.port)).await.unwrap();
    s.write_all(&[0]).await.unwrap();
    let n = timeout(Duration::from_secs(2), s.read(&mut b)).await.unwrap().unwrap();
    assert!(n > 0);
    h.shutdown().await;
}

#[tokio::test]
async fn udp_client() {
    let h = hub(&["-d", "loopback:rate=0", "--udp"]).await;
    let port = h.udp_port.unwrap();
    let s = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    s.connect(("127.0.0.1", port)).await.unwrap();
    s.send(&[0]).await.unwrap();
    s.send(&wire_port(61, b"big\r")).await.unwrap();
    let mut rx = Rx::default();
    let mut buf = vec![0u8; 65536];
    let mut largest = 0;
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end && rx.count_port(61) == 0 {
        if let Ok(Ok(n)) = timeout(Duration::from_millis(200), s.recv(&mut buf)).await {
            largest = largest.max(n);
            rx.feed(&buf[..n]);
        }
    }
    assert!(is_info(&rx.frames[0]));
    assert_eq!(rx.count_port(61), 1);
    assert!(largest <= 1452, "datagrams fit the limit: {largest}");
    // A second UDP source is ignored while the first is active.
    let s2 = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    s2.send_to(&[0], ("127.0.0.1", port)).await.unwrap();
    assert!(timeout(Duration::from_millis(500), s2.recv(&mut buf)).await.is_err());
    h.shutdown().await;
}

fn self_signed(dir: &std::path::Path) -> (String, String, rustls::pki_types::CertificateDer<'static>) {
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = dir.join("cert.pem");
    let key = dir.join("key.pem");
    std::fs::write(&cert, ck.cert.pem()).unwrap();
    std::fs::write(&key, ck.signing_key.serialize_pem()).unwrap();
    (cert.to_str().unwrap().into(), key.to_str().unwrap().into(), ck.cert.der().clone())
}

fn client_tls(ca: rustls::pki_types::CertificateDer<'static>) -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca).unwrap();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

#[tokio::test]
async fn tls_raw_and_wss_on_the_same_port() {
    let d = tempfile::tempdir().unwrap();
    let (cert, key, der) = self_signed(d.path());
    let h = hub(&["-d", "loopback:rate=0", "--tls-cert", &cert, "--tls-key", &key]).await;
    let cfg = client_tls(der);
    // Raw protocol over TLS.
    let tcp = TcpStream::connect(("127.0.0.1", h.port)).await.unwrap();
    let conn = tokio_rustls::TlsConnector::from(cfg.clone());
    let mut t = conn.connect("localhost".try_into().unwrap(), tcp).await.unwrap();
    t.write_all(&[0]).await.unwrap();
    t.write_all(&wire_port(61, b"echo over tls\r")).await.unwrap();
    let mut rx = Rx::default();
    let mut buf = vec![0u8; 65536];
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end && !rx.port_text(61).contains("over tls") {
        if let Ok(Ok(n)) = timeout(Duration::from_millis(200), t.read(&mut buf)).await {
            rx.feed(&buf[..n]);
        }
    }
    assert!(rx.port_text(61).contains("over tls"));
    drop(t);
    tokio::time::sleep(Duration::from_millis(200)).await;
    // WebSocket over TLS.
    let tcp = TcpStream::connect(("127.0.0.1", h.port)).await.unwrap();
    let (mut ws, _) = tokio_tungstenite::client_async_tls_with_config(
        format!("wss://localhost:{}/", h.port),
        tcp,
        None,
        Some(tokio_tungstenite::Connector::Rustls(cfg)),
    )
    .await
    .unwrap();
    let m = timeout(Duration::from_secs(2), ws.next()).await.unwrap();
    assert!(matches!(m, Some(Ok(Message::Binary(_)))), "{m:?}");
    h.shutdown().await;
}
