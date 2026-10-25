//! Client side: one TCP port for raw, WebSocket and TLS (sniffed from the first byte), an optional
//! WebSocket-only port, and an optional UDP client (hub plan §4).

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};
use uclog_link::WS_BUSY;
use uclog_proto::MAX_WIRE;

use crate::admission::{Answer, Decision, Prompter, Rules};
use crate::core::{client_session, ClientConn, Shared, SlotGuard};

const SNIFF_TIMEOUT: Duration = Duration::from_millis(500);
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);
/// TLS and WebSocket handshakes (before admission) must finish within this, so peers that
/// connect and stall do not hold sockets open.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// TCP/WebSocket writes carry up to this many bytes of whole frames.
const STREAM_CHUNK: usize = 16 * 1024;

pub struct Ctx {
    pub shared: Arc<Shared>,
    pub rules: Rules,
    pub prompter: Prompter,
    pub tls: Option<TlsAcceptor>,
}

/// A stream with some bytes already read in front of it.
struct Prefixed<S> {
    prefix: Vec<u8>,
    pos: usize,
    inner: S,
}

impl<S: AsyncRead + Unpin> AsyncRead for Prefixed<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        if self.pos < self.prefix.len() {
            let n = (self.prefix.len() - self.pos).min(buf.remaining());
            let p = self.pos;
            buf.put_slice(&self.prefix[p..p + n]);
            self.pos += n;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Prefixed<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, b: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, b)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Admission (hub plan §4.3). Returns the active slot, or `None` (rejected; already logged).
async fn admit(ctx: &Ctx, guard: SlotGuard, peer: SocketAddr, kind: &str, origin: Option<&str>) -> Option<SlotGuard> {
    match ctx.rules.decide(peer.ip(), origin) {
        Decision::Accept => {}
        Decision::Ask(why) => {
            if !ctx.prompter.interactive() {
                warn!(
                    "rejected {kind} connection from {peer} ({why}); allow it with --allow-from {} or --allow-all{}",
                    peer.ip(),
                    if origin.is_some() { ", or --allow-origin for a browser page" } else { "" }
                );
                return None;
            }
            let q = format!("Incoming {kind} connection from {peer} ({why}) — accept?");
            match ctx.prompter.ask(q, APPROVAL_TIMEOUT).await {
                Answer::Yes => {}
                Answer::Always => {
                    if let Err(e) = ctx.rules.remember(peer.ip()) {
                        warn!("could not save the allow-list: {e}");
                    }
                }
                Answer::No => {
                    info!("rejected {kind} connection from {peer}");
                    return None;
                }
            }
        }
    }
    guard.activate();
    Some(guard)
}

/// Accept loop for the main port (raw / WebSocket / TLS).
pub async fn serve_tcp(ctx: Arc<Ctx>, l: TcpListener, ws_only: bool) {
    loop {
        let (s, peer) = match l.accept().await {
            Ok(x) => x,
            Err(e) => {
                warn!("accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let _ = s.set_nodelay(true);
        let ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_tcp(ctx, s, peer, ws_only).await {
                debug!("connection from {peer}: {e}");
            }
        });
    }
}

async fn first_byte<S: AsyncRead + Unpin>(s: &mut S) -> io::Result<Option<u8>> {
    let mut b = [0u8; 1];
    match tokio::time::timeout(SNIFF_TIMEOUT, s.read(&mut b)).await {
        Err(_) => Ok(None), // a raw client that has not sent its first pulse yet
        Ok(Ok(0)) => Err(io::ErrorKind::UnexpectedEof.into()),
        Ok(Ok(_)) => Ok(Some(b[0])),
        Ok(Err(e)) => Err(e),
    }
}

async fn handle_tcp(ctx: Arc<Ctx>, s: TcpStream, peer: SocketAddr, ws_only: bool) -> io::Result<()> {
    if ws_only {
        return ws(ctx, s, peer, "ws").await;
    }
    let mut pk = [0u8; 1];
    let first = match tokio::time::timeout(SNIFF_TIMEOUT, s.peek(&mut pk)).await {
        Ok(Ok(0)) => return Ok(()),
        Ok(Ok(_)) => Some(pk[0]),
        Ok(Err(e)) => return Err(e),
        Err(_) => None,
    };
    match first {
        Some(0x16) => {
            let Some(acc) = ctx.tls.clone() else {
                info!("TLS connection from {peer} refused: the hub has no --tls-cert");
                return Ok(());
            };
            let mut t = tokio::time::timeout(HANDSHAKE_TIMEOUT, acc.accept(s))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TLS handshake"))??;
            match first_byte(&mut t).await? {
                Some(b'G') => ws(ctx, Prefixed { prefix: vec![b'G'], pos: 0, inner: t }, peer, "wss").await,
                Some(b) if b != 0 => refuse_raw(peer, "tls", b),
                b => raw(ctx, Prefixed { prefix: b.into_iter().collect(), pos: 0, inner: t }, peer, "tls").await,
            }
        }
        Some(b'G') => ws(ctx, s, peer, "ws").await,
        Some(b) if b != 0 => refuse_raw(peer, "tcp", b),
        _ => raw(ctx, s, peer, "tcp").await,
    }
}

/// A raw client's first byte MUST be `00` (protocol.md §4.5). Anything else is refused: an
/// HTTP POST from a web page would otherwise reach the device with no Origin check (its body
/// frames survive the scanner, which drops the header bytes before the first `00`).
fn refuse_raw(peer: SocketAddr, kind: &str, b: u8) -> io::Result<()> {
    info!("{kind} connection from {peer} refused: first byte {b:#04x}, not 00 (raw clients start with a pulse)");
    Ok(())
}

/// Raw byte stream (TCP or TLS).
async fn raw<S>(ctx: Arc<Ctx>, s: S, peer: SocketAddr, kind: &'static str) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Some(guard) = ctx.shared.claim(peer) else {
        info!("{kind} connection from {peer}: busy");
        // Closing without data tells a raw client the hub is busy. Read what it sent first, so
        // the close is a clean FIN rather than a reset.
        let mut s = s;
        let mut b = [0u8; 256];
        let _ = tokio::time::timeout(Duration::from_millis(100), s.read(&mut b)).await;
        let _ = s.shutdown().await;
        let _ = tokio::time::timeout(Duration::from_millis(200), s.read(&mut b)).await;
        return Ok(());
    };
    let (mut r, mut w) = tokio::io::split(s);
    let (in_tx, in_rx) = mpsc::channel(64);
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    // Read while admission is pending, so the client's pulses do not pile up.
    let reader = tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match r.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    if in_tx.send(buf[..n].to_vec()).await.is_err() {
                        return;
                    }
                }
            }
        }
    });
    let Some(guard) = admit(&ctx, guard, peer, kind, None).await else {
        reader.abort();
        return Ok(());
    };
    let writer = tokio::spawn(async move {
        while let Some(d) = out_rx.recv().await {
            if w.write_all(&d).await.is_err() {
                return;
            }
        }
        let _ = w.shutdown().await;
    });
    let conn = ClientConn { peer, kind, rx: in_rx, tx: out_tx, max_chunk: STREAM_CHUNK };
    client_session(ctx.shared.clone(), guard, conn).await;
    reader.abort();
    let _ = writer.await;
    Ok(())
}

/// WebSocket (plain or over TLS). Paths `/` and `/uclog`.
// The handshake callback's error type is fixed by tungstenite.
#[allow(clippy::result_large_err)]
async fn ws<S>(ctx: Arc<Ctx>, s: S, peer: SocketAddr, kind: &'static str) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut origin: Option<String> = None;
    let cb = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        origin = req.headers().get("origin").and_then(|v| v.to_str().ok()).map(str::to_string);
        if !matches!(req.uri().path(), "/" | "/uclog") {
            let mut e = ErrorResponse::new(Some("not found: use / or /uclog".into()));
            *e.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::NOT_FOUND;
            return Err(e);
        }
        Ok(resp)
    };
    let mut wsock = tokio::time::timeout(HANDSHAKE_TIMEOUT, tokio_tungstenite::accept_hdr_async(s, cb))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "WebSocket handshake"))?
        .map_err(io::Error::other)?;
    let Some(guard) = ctx.shared.claim(peer) else {
        info!("{kind} connection from {peer}: busy");
        let _ = wsock.close(Some(CloseFrame { code: CloseCode::from(WS_BUSY), reason: "busy".into() })).await;
        return Ok(());
    };
    let (mut w, mut r) = wsock.split();
    let (in_tx, in_rx) = mpsc::channel(64);
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    let reader = tokio::spawn(async move {
        while let Some(m) = r.next().await {
            match m {
                Ok(Message::Binary(b)) => {
                    if in_tx.send(b.to_vec()).await.is_err() {
                        return;
                    }
                }
                Ok(Message::Text(_)) => {
                    warn!("WebSocket client {peer} sent a text message (protocol error)");
                    return;
                }
                Ok(Message::Close(_)) | Err(_) => return,
                Ok(_) => {} // ping/pong handled by tungstenite
            }
        }
    });
    let Some(guard) = admit(&ctx, guard, peer, kind, origin.as_deref()).await else {
        reader.abort();
        let _ = w.send(Message::Close(Some(CloseFrame { code: CloseCode::Policy, reason: "rejected".into() }))).await;
        return Ok(());
    };
    let writer = tokio::spawn(async move {
        while let Some(d) = out_rx.recv().await {
            if w.send(Message::binary(d)).await.is_err() {
                return;
            }
        }
        let _ = w.send(Message::Close(None)).await;
    });
    let conn = ClientConn { peer, kind, rx: in_rx, tx: out_tx, max_chunk: STREAM_CHUNK };
    client_session(ctx.shared.clone(), guard, conn).await;
    reader.abort();
    let _ = writer.await;
    Ok(())
}

/// UDP client side: the client is the source address of the first datagram (hub plan §4.2).
pub async fn serve_udp(ctx: Arc<Ctx>, sock: UdpSocket) {
    let sock = Arc::new(sock);
    let v6 = sock.local_addr().map(|a| a.is_ipv6()).unwrap_or(false);
    if let Err(e) = uclog_link::set_dont_fragment(&sock, v6) {
        warn!("UDP: could not set Don't Fragment: {e}");
    }
    let mut buf = vec![0u8; 65536];
    let mut active: Option<(SocketAddr, mpsc::Sender<Vec<u8>>)> = None;
    loop {
        let (n, from) = match sock.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(e) => {
                debug!("UDP recv: {e}");
                continue;
            }
        };
        if let Some((a, tx)) = &active {
            if *a == from {
                if tx.send(buf[..n].to_vec()).await.is_err() {
                    active = None; // session over; this datagram may start a new one below
                } else {
                    continue;
                }
            } else if !tx.is_closed() {
                continue; // busy: ignored
            } else {
                active = None;
            }
        }
        let Some(guard) = ctx.shared.claim(from) else { continue };
        let (in_tx, in_rx) = mpsc::channel(64);
        let _ = in_tx.try_send(buf[..n].to_vec());
        active = Some((from, in_tx));
        let ctx = ctx.clone();
        let sock = sock.clone();
        tokio::spawn(async move {
            let Some(guard) = admit(&ctx, guard, from, "udp", None).await else { return };
            let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
            let s2 = sock.clone();
            let writer = tokio::spawn(async move {
                while let Some(d) = out_rx.recv().await {
                    if let Err(e) = s2.send_to(&d, from).await {
                        warn!("UDP to {from}: {e}");
                        if uclog_link::is_too_big(&e) {
                            return;
                        }
                    }
                }
            });
            let conn = ClientConn { peer: from, kind: "udp", rx: in_rx, tx: out_tx, max_chunk: MAX_WIRE };
            client_session(ctx.shared.clone(), guard, conn).await;
            writer.abort();
        });
    }
}
