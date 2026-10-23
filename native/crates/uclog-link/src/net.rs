//! UDP (device), TCP and WebSocket (hub) links (protocol.md §4.4, §4.5).

use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio_tungstenite::tungstenite::Message;

use crate::{channels, CloseReason, Link, LinkError, LinkEvent, LinkSpec};

/// WebSocket close code a hub uses when another client is connected.
pub const WS_BUSY: u16 = 4001;

pub(crate) async fn open_udp(spec: LinkSpec, addr: &str) -> Result<Link, LinkError> {
    let sock = UdpSocket::bind(if addr.starts_with('[') { "[::]:0" } else { "0.0.0.0:0" }).await?;
    sock.connect(addr).await?;
    let sock = std::sync::Arc::new(sock);
    let (tx, mut txr, ev, evr) = channels();
    let r = sock.clone();
    let ev2 = ev.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        loop {
            tokio::select! {
                res = r.recv(&mut buf) => match res {
                    Ok(n) => { if ev.send(LinkEvent::Data(buf[..n].to_vec())).await.is_err() { return; } }
                    // ICMP port unreachable etc.: keep listening, liveness will report silence.
                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {}
                    Err(e) => { let _ = ev.send(LinkEvent::Closed(CloseReason::Error(e.to_string()))).await; return; }
                },
                _ = ev.closed() => return,
            }
        }
    });
    tokio::spawn(async move {
        while let Some(d) = txr.recv().await {
            // Each write is whole frames; one datagram per write.
            if let Err(e) = sock.send(&d).await {
                if e.kind() != std::io::ErrorKind::ConnectionRefused {
                    let _ = ev2.send(LinkEvent::Closed(CloseReason::Error(e.to_string()))).await;
                    return;
                }
            }
        }
    });
    Ok(Link { description: format!("UDP {addr}"), spec, tx, rx: evr })
}

pub(crate) async fn open_tcp(spec: LinkSpec, addr: &str) -> Result<Link, LinkError> {
    let s = TcpStream::connect(addr).await?;
    let _ = s.set_nodelay(true);
    let (mut r, mut w) = s.into_split();
    let (tx, mut txr, ev, evr) = channels();
    let ev2 = ev.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 16384];
        let mut got_data = false;
        loop {
            tokio::select! {
                res = r.read(&mut buf) => match res {
                    Ok(0) => {
                        // A hub closes before any data when another client is connected.
                        let reason = if got_data { CloseReason::Eof } else { CloseReason::HubBusy };
                        let _ = ev.send(LinkEvent::Closed(reason)).await;
                        return;
                    }
                    Ok(n) => { got_data = true; if ev.send(LinkEvent::Data(buf[..n].to_vec())).await.is_err() { return; } }
                    Err(e) => { let _ = ev.send(LinkEvent::Closed(CloseReason::Error(e.to_string()))).await; return; }
                },
                _ = ev.closed() => return,
            }
        }
    });
    tokio::spawn(async move {
        while let Some(d) = txr.recv().await {
            if let Err(e) = w.write_all(&d).await {
                let _ = ev2.send(LinkEvent::Closed(CloseReason::Error(e.to_string()))).await;
                return;
            }
        }
        let _ = w.shutdown().await;
    });
    Ok(Link { description: format!("hub {addr}"), spec, tx, rx: evr })
}

pub(crate) async fn open_ws(spec: LinkSpec, url: &str) -> Result<Link, LinkError> {
    let (ws, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| LinkError::Ws(e.to_string()))?;
    let (mut w, mut r) = ws.split();
    let (tx, mut txr, ev, evr) = channels();
    let ev2 = ev.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                m = r.next() => match m {
                    Some(Ok(Message::Binary(b))) => { if ev.send(LinkEvent::Data(b.to_vec())).await.is_err() { return; } }
                    Some(Ok(Message::Close(f))) => {
                        let reason = match f {
                            Some(f) if u16::from(f.code) == WS_BUSY => CloseReason::HubBusy,
                            Some(f) => CloseReason::Error(format!("closed by hub: {} {}", u16::from(f.code), f.reason)),
                            None => CloseReason::Eof,
                        };
                        let _ = ev.send(LinkEvent::Closed(reason)).await;
                        return;
                    }
                    Some(Ok(Message::Text(_))) => {
                        let _ = ev.send(LinkEvent::Closed(CloseReason::Error("protocol error: text message from hub".into()))).await;
                        return;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => { let _ = ev.send(LinkEvent::Closed(CloseReason::Error(e.to_string()))).await; return; }
                    None => { let _ = ev.send(LinkEvent::Closed(CloseReason::Eof)).await; return; }
                },
                _ = ev.closed() => return,
            }
        }
    });
    tokio::spawn(async move {
        while let Some(d) = txr.recv().await {
            if let Err(e) = w.send(Message::binary(d)).await {
                let _ = ev2.send(LinkEvent::Closed(CloseReason::Error(e.to_string()))).await;
                return;
            }
        }
        let _ = w.close().await;
    });
    Ok(Link { description: format!("hub {url}"), spec, tx, rx: evr })
}
