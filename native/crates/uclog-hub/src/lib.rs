//! `uclog hub`: owns one device and serves one client at a time over raw TCP, WebSocket (both on
//! one port, optionally TLS) and UDP (`docs/plans/01-hub-server.md`).
//!
//! The hub does not decode logs. It frames the stream, peeks at port numbers, keeps a backlog for
//! clients that connect later, answers the time syscalls itself, and routes the other syscalls.

pub mod admission;
pub mod announce;
pub mod backlog;
pub mod config;
pub mod core;
pub mod list;
pub mod listener;
pub mod syscalls;

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::oneshot;
use tracing::{info, warn};
use uclog_link::{discover, LinkSpec};

pub use config::HubArgs;

use crate::admission::{Prompter, Rules};
use crate::announce::{Identity, Mdns, Registry};
use crate::config::OnOff;
use crate::core::{device_task, Opener, Shared};
use crate::listener::{serve_tcp, serve_udp, Ctx};
use crate::syscalls::Router;

/// Default client port and the range tried when it is taken (hub plan §8).
pub const DEFAULT_PORTS: std::ops::RangeInclusive<u16> = 9000..=9019;

/// A running hub.
pub struct Running {
    pub port: u16,
    pub ws_port: Option<u16>,
    pub udp_port: Option<u16>,
    pub identity: Identity,
    pub shared: Arc<Shared>,
    stop: Option<oneshot::Sender<()>>,
    done: tokio::task::JoinHandle<()>,
    stdin_eof: Option<oneshot::Receiver<()>>,
}

impl Running {
    /// Wait for Ctrl-C (or stdin EOF with `--exit-on-stdin-eof`), then shut down.
    pub async fn wait(mut self) {
        let eof = async {
            match self.stdin_eof.take() {
                Some(rx) => {
                    let _ = rx.await;
                }
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => info!("interrupted"),
            _ = eof => info!("stdin closed"),
        }
        self.shutdown().await;
    }

    pub async fn shutdown(mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        let _ = (&mut self.done).await;
    }
}

async fn bind_tcp(bind: std::net::IpAddr, port: Option<u16>) -> anyhow::Result<TcpListener> {
    match port {
        Some(p) => {
            TcpListener::bind(SocketAddr::new(bind, p)).await.with_context(|| format!("listening on {bind}:{p}"))
        }
        None => {
            for p in DEFAULT_PORTS {
                if let Ok(l) = TcpListener::bind(SocketAddr::new(bind, p)).await {
                    if p != *DEFAULT_PORTS.start() {
                        warn!("port {} is in use; using {p}", DEFAULT_PORTS.start());
                    }
                    return Ok(l);
                }
            }
            bail!("ports {}–{} are all in use; choose one with --port", DEFAULT_PORTS.start(), DEFAULT_PORTS.end())
        }
    }
}

fn tls_acceptor(a: &HubArgs) -> anyhow::Result<Option<tokio_rustls::TlsAcceptor>> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let (Some(cert), Some(key)) = (&a.tls_cert, &a.tls_key) else { return Ok(None) };
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert)
        .with_context(|| format!("reading {}", cert.display()))?
        .collect::<Result<_, _>>()
        .with_context(|| format!("parsing {}", cert.display()))?;
    let key = PrivateKeyDer::from_pem_file(key).with_context(|| format!("reading {}", key.display()))?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("TLS certificate/key")?;
    Ok(Some(tokio_rustls::TlsAcceptor::from(Arc::new(cfg))))
}

fn resolve(a: &HubArgs) -> anyhow::Result<LinkSpec> {
    let spec = match &a.device {
        LinkSpec::Auto => discover::resolve_auto(&a.usb_filter).map_err(|(msg, list)| {
            let mut m = msg;
            for d in list {
                m.push_str(&format!("\n  {:<40} {} {}", d.uri, d.name, d.serial));
            }
            anyhow!(m)
        })?,
        s => s.clone(),
    };
    if spec.is_hub() {
        bail!("a hub serves a device, not another hub ({spec})");
    }
    Ok(spec.with_baud(a.baud))
}

/// The TCP listener and, with UDP on, its socket (on the TCP port unless `--udp-port`). With
/// `--port 0` a port whose UDP side is refused (Windows reserves ranges for UDP alone) is given
/// up for another.
async fn bind_tcp_udp(a: &HubArgs) -> anyhow::Result<(TcpListener, Option<UdpSocket>)> {
    let mut tries = 0;
    loop {
        let tcp = bind_tcp(a.bind, a.port).await?;
        if !a.udp {
            return Ok((tcp, None));
        }
        let p = a.udp_port.unwrap_or(tcp.local_addr()?.port());
        match UdpSocket::bind(SocketAddr::new(a.bind, p)).await {
            Ok(u) => return Ok((tcp, Some(u))),
            Err(_) if a.port == Some(0) && a.udp_port.is_none() && tries < 10 => tries += 1,
            Err(e) => return Err(e).with_context(|| format!("UDP port {p}")),
        }
    }
}

/// Start the hub; returns once it is listening.
pub async fn start(a: HubArgs, verbose: u8) -> anyhow::Result<Running> {
    let spec = resolve(&a)?;
    let files_dir = if a.syscalls {
        Some(match &a.files_dir {
            Some(d) => d.clone(),
            None => std::env::current_dir()?,
        })
    } else {
        None
    };
    let router = Router::new(files_dir.clone(), a.client_syscalls == OnOff::On);
    let (shared, to_device) = Shared::new(a.backlog, router, a.max_frame, a.slow_client, a.client_timeout());

    let (tcp, udp) = bind_tcp_udp(&a).await?;
    let port = tcp.local_addr()?.port();
    let ws = match a.ws_port {
        Some(p) => Some(TcpListener::bind(SocketAddr::new(a.bind, p)).await.with_context(|| format!("--ws-port {p}"))?),
        None => None,
    };
    let ws_port = ws.as_ref().map(|l| l.local_addr().map(|a| a.port())).transpose()?;
    let udp_port = udp.as_ref().map(|s| s.local_addr().map(|a| a.port())).transpose()?;
    let tls = tls_acceptor(&a)?;

    // Identity for mDNS and the registry.
    let dev = discover::local_devices(&a.usb_filter).into_iter().find(|d| d.uri == spec.to_string());
    let interface = announce::interface_name(&spec);
    let identity = Identity {
        name: format!("{}-{interface}", announce::hostname()),
        interface,
        kind: match &spec {
            LinkSpec::Serial { .. } => "serial",
            LinkSpec::Hid { .. } => "hid",
            LinkSpec::Ble { .. } => "ble",
            LinkSpec::Udp { .. } => "udp",
            _ => "loopback",
        }
        .into(),
        device: spec.to_string(),
        serial_number: dev.as_ref().map(|d| d.serial.clone()).unwrap_or_default(),
        description: dev.as_ref().map(|d| d.name.clone()).unwrap_or_default(),
        port,
        ws_port,
        udp_port,
        bind: a.bind.to_string(),
        tls: tls.is_some(),
    };

    let (eof_tx, eof_rx) = oneshot::channel();
    let interactive = std::io::stdin().is_terminal();
    let prompter = Prompter::start(interactive, a.exit_on_stdin_eof, a.exit_on_stdin_eof.then_some(eof_tx));
    let allow_file =
        directories::ProjectDirs::from("ca", "unitcircle", "uclog").map(|d| d.config_dir().join("hub.allow"));
    let rules = Rules::new(a.allow_all, a.allow_from.clone(), a.allow_origin.clone(), allow_file);
    let ctx = Arc::new(Ctx { shared: shared.clone(), rules, prompter, tls });

    let auto = matches!(a.device, LinkSpec::Auto);
    let (filters, baud) = (a.usb_filter.clone(), a.baud);
    let spec2 = spec.clone();
    let opener: Opener = Box::new(move || {
        // `auto`: find the device again (its path can change when it re-enumerates).
        let s = if auto {
            discover::resolve_auto(&filters).map(|s| s.with_baud(baud)).unwrap_or_else(|_| spec2.clone())
        } else {
            spec2.clone()
        };
        Box::pin(async move { uclog_link::open(&s).await })
    });

    let mut tasks = vec![
        tokio::spawn(device_task(shared.clone(), opener, to_device)),
        tokio::spawn(serve_tcp(ctx.clone(), tcp, false)),
    ];
    if let Some(l) = ws {
        tasks.push(tokio::spawn(serve_tcp(ctx.clone(), l, true)));
    }
    if let Some(u) = udp {
        tasks.push(tokio::spawn(serve_udp(ctx.clone(), u)));
    }
    if verbose > 0 && a.stats_interval > 0.0 {
        tasks.push(tokio::spawn(stats(shared.clone(), Duration::from_secs_f64(a.stats_interval))));
    }

    info!(
        "hub for {} on {}:{port}{}{}{} ({})",
        identity.device,
        a.bind,
        ws_port.map(|p| format!(", WebSocket also on {p}")).unwrap_or_default(),
        udp_port.map(|p| format!(", UDP {p}")).unwrap_or_default(),
        if identity.tls { ", TLS" } else { "" },
        identity.name
    );
    if let Some(d) = &files_dir {
        info!("syscalls answered by the hub, files in {}", d.display());
    }
    // Announce, and keep `busy` up to date. A client may already be connected (the listener is
    // running), so start from the current value.
    let mut mdns = if a.no_mdns { None } else { Mdns::start(identity.clone()) };
    let registry = if a.no_registry { None } else { Registry::create(identity.clone()) };
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let mut busy = shared.busy.subscribe();
    if *busy.borrow_and_update() {
        if let Some(m) = &mut mdns {
            m.publish(true);
        }
        if let Some(r) = &registry {
            r.write(true);
        }
    }
    // Ready only once the registry is written: a script reads this line and connects at once.
    if a.json_ready {
        let j = serde_json::json!({"port": port, "ws_port": ws_port, "udp_port": udp_port,
                                   "name": identity.name, "device": identity.device});
        println!("{j}");
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
    let done = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                c = busy.changed() => {
                    if c.is_err() { break; }
                    let b = *busy.borrow();
                    if let Some(m) = &mut mdns { m.publish(b); }
                    if let Some(r) = &registry { r.write(b); }
                }
            }
        }
        for t in tasks {
            t.abort();
        }
        if let Some(m) = mdns {
            let _ = tokio::task::spawn_blocking(move || m.stop()).await;
        }
        drop(registry);
    });
    Ok(Running {
        port,
        ws_port,
        udp_port,
        identity,
        shared,
        stop: Some(stop_tx),
        done,
        stdin_eof: a.exit_on_stdin_eof.then_some(eof_rx),
    })
}

/// Run until interrupted.
pub async fn run(a: HubArgs, verbose: u8) -> anyhow::Result<()> {
    start(a, verbose).await?.wait().await;
    Ok(())
}

async fn stats(shared: Arc<Shared>, every: Duration) {
    let mut t = tokio::time::interval(every);
    t.tick().await;
    loop {
        t.tick().await;
        let c = &shared.counters;
        let (frames, bytes) = shared.backlog.lock().unwrap().len();
        let s = shared.router.lock().unwrap().stats;
        info!(
            "stats: device {:?}, {} frames / {} bytes in, {} framing errors; backlog {frames} frames / {bytes} bytes; \
             to device {} frames ({} dropped); client {} sessions, {} frames out, {} dropped, {} framing errors; \
             syscalls hub {} forwarded {} replies {} EIO {}",
            *shared.state.borrow(),
            c.dev_frames.load(Relaxed),
            c.dev_bytes.load(Relaxed),
            c.dev_framing_errors.load(Relaxed),
            c.to_dev_frames.load(Relaxed),
            c.to_dev_dropped.load(Relaxed),
            c.sessions.load(Relaxed),
            c.client_frames_out.load(Relaxed),
            c.client_dropped.load(Relaxed),
            c.client_framing_errors.load(Relaxed),
            s.hub,
            s.forwarded,
            s.client_replies,
            s.eio_on_disconnect,
        );
    }
}
