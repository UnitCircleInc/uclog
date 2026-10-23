//! UDP (device), TCP and WebSocket (hub) links (protocol.md §4.4, §4.5).

use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio_tungstenite::tungstenite::Message;

use crate::{channels, CloseReason, Link, LinkError, LinkEvent, LinkSpec};

/// WebSocket close code a hub uses when another client is connected.
pub const WS_BUSY: u16 = 4001;

/// Shown when a datagram is refused as too large (protocol.md §4.4).
const TOO_BIG: &str = "UDP: datagram refused as too large (EMSGSIZE / fragmentation needed); the path MTU is \
                       below 1500, which UDP links do not support (use TCP or WebSocket to a hub)";

/// The error a datagram too large for the path gets (EMSGSIZE).
pub fn is_too_big(e: &std::io::Error) -> bool {
    #[cfg(unix)]
    let code = libc::EMSGSIZE;
    #[cfg(windows)]
    let code = windows_sys::Win32::Networking::WinSock::WSAEMSGSIZE;
    e.raw_os_error() == Some(code)
}

fn udp_error(e: std::io::Error) -> CloseReason {
    CloseReason::Error(if is_too_big(&e) { TOO_BIG.into() } else { e.to_string() })
}

/// Set Don't Fragment, so that neither this host nor a router fragments our datagrams
/// (protocol.md §4.4). Oversized datagrams then fail with EMSGSIZE instead.
#[cfg_attr(
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        windows
    )),
    allow(unused_variables)
)]
pub fn set_dont_fragment(sock: &UdpSocket, v6: bool) -> std::io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let (level, name, value) = if v6 {
        (libc::IPPROTO_IPV6, libc::IPV6_MTU_DISCOVER, libc::IPV6_PMTUDISC_DO)
    } else {
        (libc::IPPROTO_IP, libc::IP_MTU_DISCOVER, libc::IP_PMTUDISC_DO)
    };
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    let (level, name, value) = if v6 {
        (libc::IPPROTO_IPV6, libc::IPV6_DONTFRAG, 1)
    } else {
        (libc::IPPROTO_IP, if cfg!(target_os = "freebsd") { 67 } else { 28 }, 1)
        // IP_DONTFRAG
    };
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd"
    ))]
    {
        use std::os::fd::AsRawFd;
        let value: libc::c_int = value;
        // SAFETY: valid socket, option value is a c_int of the stated size.
        let r = unsafe {
            libc::setsockopt(
                sock.as_raw_fd(),
                level,
                name,
                &value as *const libc::c_int as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if r != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock as ws;
        let (level, name) =
            if v6 { (ws::IPPROTO_IPV6, ws::IPV6_DONTFRAG) } else { (ws::IPPROTO_IP, ws::IP_DONTFRAGMENT) };
        let value: u32 = 1;
        // SAFETY: valid socket, option value is a DWORD of the stated size.
        let r = unsafe {
            ws::setsockopt(
                sock.as_raw_socket() as ws::SOCKET,
                level,
                name,
                &value as *const u32 as *const u8,
                std::mem::size_of::<u32>() as i32,
            )
        };
        if r != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

pub(crate) async fn open_udp(spec: LinkSpec, addr: &str) -> Result<Link, LinkError> {
    let sock = UdpSocket::bind(if addr.starts_with('[') { "[::]:0" } else { "0.0.0.0:0" }).await?;
    sock.connect(addr).await?;
    let v6 = sock.peer_addr()?.is_ipv6();
    set_dont_fragment(&sock, v6)?;
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
                    Err(e) => { let _ = ev.send(LinkEvent::Closed(udp_error(e))).await; return; }
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
                    let _ = ev2.send(LinkEvent::Closed(udp_error(e))).await;
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
                    // A busy hub's close can arrive as a reset when our first pulse was unread.
                    Err(e) if !got_data && e.kind() == std::io::ErrorKind::ConnectionReset => {
                        let _ = ev.send(LinkEvent::Closed(CloseReason::HubBusy)).await;
                        return;
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Read an int socket option back (unix).
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn get_opt(sock: &UdpSocket, level: libc::c_int, name: libc::c_int) -> libc::c_int {
        use std::os::fd::AsRawFd;
        let mut v: libc::c_int = -1;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: valid socket and out-pointers of the stated size.
        let r = unsafe {
            libc::getsockopt(sock.as_raw_fd(), level, name, &mut v as *mut libc::c_int as *mut libc::c_void, &mut len)
        };
        assert_eq!(r, 0);
        v
    }

    #[tokio::test]
    async fn dont_fragment_is_set() {
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        set_dont_fragment(&sock, false).unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(get_opt(&sock, libc::IPPROTO_IP, libc::IP_MTU_DISCOVER), libc::IP_PMTUDISC_DO);
        #[cfg(target_os = "macos")]
        assert_eq!(get_opt(&sock, libc::IPPROTO_IP, 28), 1); // IP_DONTFRAG
        if let Ok(s6) = UdpSocket::bind("[::1]:0").await {
            set_dont_fragment(&s6, true).unwrap();
            #[cfg(target_os = "linux")]
            assert_eq!(get_opt(&s6, libc::IPPROTO_IPV6, libc::IPV6_MTU_DISCOVER), libc::IPV6_PMTUDISC_DO);
            #[cfg(target_os = "macos")]
            assert_eq!(get_opt(&s6, libc::IPPROTO_IPV6, libc::IPV6_DONTFRAG), 1);
        }
    }

    #[test]
    fn too_big_message() {
        #[cfg(unix)]
        let e = std::io::Error::from_raw_os_error(libc::EMSGSIZE);
        #[cfg(windows)]
        let e = std::io::Error::from_raw_os_error(windows_sys::Win32::Networking::WinSock::WSAEMSGSIZE);
        assert!(matches!(udp_error(e), CloseReason::Error(m) if m.contains("MTU is below 1500")));
    }
}
