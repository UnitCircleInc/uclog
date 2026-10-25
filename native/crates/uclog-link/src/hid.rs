//! HID link (protocol.md §4.3): input report 0xA1, output report 0xA2, 63-byte payloads.
//! Interrupt IN reads (`read_timeout`) on every OS, including Windows (plans issue 7).

use std::ffi::CString;
use std::sync::mpsc::{self, TryRecvError};
use std::sync::OnceLock;

use uclog_proto::link::{hid_reports, HID_INTERFACE, HID_PAYLOAD, HID_REPORT_IN, HID_VID};

use crate::{channels, CloseReason, Link, LinkError, LinkEvent, LinkSpec};

type Job = Box<dyn FnOnce(Result<&mut hidapi::HidApi, String>) + Send>;

/// Run `f` on the one thread that owns the process' `HidApi`.
///
/// hidapi keeps one device manager per process. On macOS it is scheduled on the run loop of the
/// thread that first used it, so every later call must come from a thread that is still alive: a
/// `HidApi` created on a short-lived thread (e.g. a tokio blocking thread, which exits when
/// idle) leaves the next enumeration calling into a dead run loop, and CoreFoundation traps.
fn with_api<R: Send + 'static>(f: impl FnOnce(Result<&mut hidapi::HidApi, String>) -> R + Send + 'static) -> R {
    static JOBS: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let jobs = JOBS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("hidapi".into())
            .spawn(move || {
                let mut api = hidapi::HidApi::new().map_err(|e| e.to_string());
                for job in rx {
                    job(api.as_mut().map_err(|e| e.clone()));
                }
            })
            .expect("spawning the hidapi thread");
        tx
    });
    let (rtx, rrx) = mpsc::sync_channel(1);
    let job: Job = Box::new(move |api| {
        let _ = rtx.send(f(api));
    });
    jobs.send(job).expect("the hidapi thread runs for the life of the process");
    rrx.recv().expect("the hidapi thread answers every job")
}

/// Find the device path for an id that is either a path or a serial number.
fn find(api: &hidapi::HidApi, id: &str) -> Option<(CString, String)> {
    api.device_list()
        .filter(|d| d.vendor_id() == HID_VID && d.interface_number() == HID_INTERFACE)
        .find(|d| d.path().to_str().ok() == Some(id) || d.serial_number() == Some(id))
        .map(|d| {
            let desc = format!("HID {} {}", d.product_string().unwrap_or("?"), d.serial_number().unwrap_or(""));
            (d.path().to_owned(), desc)
        })
}

pub(crate) async fn open(spec: LinkSpec, id: &str) -> Result<Link, LinkError> {
    let (tx, mut txr, ev, evr) = channels();
    let id = id.to_string();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    // hidapi handles are not Sync: one thread does both reads and writes.
    let name = format!("hid {id}");
    std::thread::Builder::new().name(name).spawn(move || {
        let opened = with_api(move |api| {
            let api = api.map_err(LinkError::Hid)?;
            let _ = api.refresh_devices();
            let (path, desc) = find(api, &id).ok_or_else(|| LinkError::NotFound(format!("no HID device {id}")))?;
            let dev = api.open_path(&path).map_err(|e| LinkError::Hid(e.to_string()))?;
            Ok::<_, LinkError>((dev, desc))
        });
        let dev = match opened {
            Ok((dev, desc)) => {
                let _ = ready_tx.send(Ok(desc));
                dev
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };
        // Bridge the async sender to this thread.
        let (wtx, wrx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            while let Some(d) = txr.blocking_recv() {
                if wtx.send(d).is_err() {
                    return;
                }
            }
        });
        let mut buf = [0u8; HID_PAYLOAD + 1];
        // One read, waiting up to `ms`. Ok(false): nothing arrived; Err: the link is gone.
        let mut read = |ms: i32| -> Result<bool, ()> {
            match dev.read_timeout(&mut buf, ms) {
                Ok(0) => {
                    if ev.is_closed() {
                        return Err(());
                    }
                    Ok(false)
                }
                Ok(n) => {
                    // Some platforms strip the report ID for single-report devices; accept both.
                    let data = if buf[0] == HID_REPORT_IN { &buf[1..n] } else { &buf[..n] };
                    ev.blocking_send(LinkEvent::Data(data.to_vec())).map(|_| true).map_err(|_| ())
                }
                Err(e) => {
                    let _ = ev.blocking_send(LinkEvent::Closed(CloseReason::Detached(e.to_string())));
                    Err(())
                }
            }
        };
        loop {
            loop {
                match wrx.try_recv() {
                    Ok(data) => {
                        for r in hid_reports(&data) {
                            if let Err(e) = dev.write(&r) {
                                let _ = ev.blocking_send(LinkEvent::Closed(CloseReason::Detached(e.to_string())));
                                return;
                            }
                            // Take what arrived meanwhile: a write waits while the device's RX is
                            // full, and hidapi drops the oldest input reports beyond 30 queued
                            // (macOS), which lost frames during long host → device bursts.
                            loop {
                                match read(0) {
                                    Ok(true) => {}
                                    Ok(false) => break,
                                    Err(()) => return,
                                }
                            }
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            if read(5).is_err() {
                return;
            }
        }
    })?;
    let description = ready_rx.await.map_err(|_| LinkError::Hid("HID thread exited".into()))??;
    Ok(Link { spec, description, tx, rx: evr })
}

/// HID devices matching the filter.
pub(crate) fn list() -> Vec<crate::Device> {
    with_api(|api| {
        let Ok(api) = api else { return vec![] };
        let _ = api.refresh_devices();
        list_from(api)
    })
}

fn list_from(api: &hidapi::HidApi) -> Vec<crate::Device> {
    let mut v: Vec<_> = api
        .device_list()
        .filter(|d| d.vendor_id() == HID_VID && d.interface_number() == HID_INTERFACE)
        .map(|d| {
            let id = d
                .serial_number()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| d.path().to_string_lossy().into_owned());
            crate::Device {
                kind: crate::Kind::Hid,
                uri: format!("hid:{id}"),
                name: d.product_string().unwrap_or("HID device").to_string(),
                serial: d.serial_number().unwrap_or("").to_string(),
                description: format!(
                    "{:04x}:{:04x} {}",
                    d.vendor_id(),
                    d.product_id(),
                    d.manufacturer_string().unwrap_or("")
                ),
                busy: None,
            }
        })
        .collect();
    v.sort_by(|a, b| a.uri.cmp(&b.uri));
    v.dedup_by(|a, b| a.uri == b.uri);
    v
}

#[cfg(test)]
mod tests {
    /// Listing from a thread that has since exited, then from other threads (what the Connect
    /// lists do via tokio blocking threads). This trapped on macOS when each call created its own
    /// `HidApi` on the calling thread.
    #[test]
    fn list_from_short_lived_threads() {
        for _ in 0..3 {
            std::thread::spawn(super::list).join().unwrap();
        }
        let _ = super::list();
    }
}
