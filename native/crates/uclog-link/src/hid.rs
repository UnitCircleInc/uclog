//! HID link (protocol.md §4.3): input report 0xA1, output report 0xA2, 63-byte payloads.
//! Interrupt IN reads (`read_timeout`) on every OS, including Windows (plans issue 7).

use std::ffi::CString;
use std::sync::mpsc::TryRecvError;

use uclog_proto::link::{hid_reports, HID_INTERFACE, HID_PAYLOAD, HID_REPORT_IN, HID_VID};

use crate::{channels, CloseReason, Link, LinkError, LinkEvent, LinkSpec};

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
    std::thread::Builder::new().name(format!("hid {id}")).spawn(move || {
        let opened = (|| {
            let api = hidapi::HidApi::new().map_err(|e| LinkError::Hid(e.to_string()))?;
            let (path, desc) = find(&api, &id).ok_or_else(|| LinkError::NotFound(format!("no HID device {id}")))?;
            let dev = api.open_path(&path).map_err(|e| LinkError::Hid(e.to_string()))?;
            Ok::<_, LinkError>((dev, desc))
        })();
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
        loop {
            loop {
                match wrx.try_recv() {
                    Ok(data) => {
                        for r in hid_reports(&data) {
                            if let Err(e) = dev.write(&r) {
                                let _ = ev.blocking_send(LinkEvent::Closed(CloseReason::Detached(e.to_string())));
                                return;
                            }
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            match dev.read_timeout(&mut buf, 5) {
                Ok(0) => {
                    if ev.is_closed() {
                        return;
                    }
                }
                Ok(n) => {
                    // Some platforms strip the report ID for single-report devices; accept both.
                    let data = if buf[0] == HID_REPORT_IN { &buf[1..n] } else { &buf[..n] };
                    if ev.blocking_send(LinkEvent::Data(data.to_vec())).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let _ = ev.blocking_send(LinkEvent::Closed(CloseReason::Detached(e.to_string())));
                    return;
                }
            }
        }
    })?;
    let description = ready_rx.await.map_err(|_| LinkError::Hid("HID thread exited".into()))??;
    Ok(Link { spec, description, tx, rx: evr })
}

/// HID devices matching the filter.
pub(crate) fn list() -> Vec<crate::Device> {
    let Ok(api) = hidapi::HidApi::new() else { return vec![] };
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
