//! BLE link (protocol.md §4.2). Notifications in; writes chunked to the negotiated MTU − 3
//! (the device copes with any chunking, D18).

use std::time::Duration;

use btleplug::api::{BDAddr, Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use uuid::Uuid;

use crate::{channels, CloseReason, Device, Kind, Link, LinkError, LinkEvent, LinkSpec};

fn uuid() -> Uuid {
    Uuid::parse_str(uclog_proto::link::BLE_UUID).unwrap()
}

fn e(x: impl std::fmt::Display) -> LinkError {
    LinkError::Ble(x.to_string())
}

async fn adapter() -> Result<Adapter, LinkError> {
    let m = Manager::new().await.map_err(e)?;
    m.adapters().await.map_err(e)?.into_iter().next().ok_or_else(|| LinkError::Ble("no Bluetooth adapter".into()))
}

/// Scan for devices advertising the ucLog service.
pub(crate) async fn scan(dur: Duration) -> Result<Vec<(Peripheral, Device)>, LinkError> {
    let a = adapter().await?;
    a.start_scan(ScanFilter { services: vec![uuid()] }).await.map_err(e)?;
    tokio::time::sleep(dur).await;
    let _ = a.stop_scan().await;
    let mut out = vec![];
    for p in a.peripherals().await.map_err(e)? {
        let props = p.properties().await.ok().flatten();
        let Some(props) = props else { continue };
        if !props.services.contains(&uuid()) {
            continue;
        }
        let name = props.local_name.clone().or(props.advertisement_name.clone()).unwrap_or_else(|| "BLE device".into());
        let id = p.id().to_string();
        // macOS does not reveal addresses (it reports 00:00:00:00:00:00): no serial then.
        let serial = if props.address == BDAddr::default() { String::new() } else { props.address.to_string() };
        out.push((
            p,
            Device {
                kind: Kind::Ble,
                uri: format!("ble:{id}"),
                name,
                serial,
                description: props.rssi.map(|r| format!("RSSI {r} dBm")).unwrap_or_default(),
                busy: None,
            },
        ));
    }
    Ok(out)
}

pub(crate) async fn open(spec: LinkSpec, id: &str) -> Result<Link, LinkError> {
    let found = scan(Duration::from_secs(4)).await?;
    let (p, dev) = found
        .into_iter()
        .find(|(_, d)| d.uri == format!("ble:{id}") || (!d.serial.is_empty() && d.serial.eq_ignore_ascii_case(id)) || d.name == id)
        .ok_or_else(|| LinkError::NotFound(format!("no BLE device {id} advertising the ucLog service")))?;
    p.connect().await.map_err(e)?;
    p.discover_services().await.map_err(e)?;
    let ch = p
        .characteristics()
        .into_iter()
        .find(|c| c.uuid == uuid())
        .ok_or_else(|| LinkError::Ble("ucLog characteristic not found".into()))?;
    p.subscribe(&ch).await.map_err(e)?;
    let mut notes = p.notifications().await.map_err(e)?;
    let write_type = if ch.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
        WriteType::WithoutResponse
    } else {
        WriteType::WithResponse
    };
    let (tx, mut txr, ev, evr) = channels();
    let ev2 = ev.clone();
    let p2 = p.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                n = notes.next() => match n {
                    Some(n) if n.uuid == uuid() => { if ev.send(LinkEvent::Data(n.value)).await.is_err() { break; } }
                    Some(_) => {}
                    None => { let _ = ev.send(LinkEvent::Closed(CloseReason::Detached("BLE disconnected".into()))).await; break; }
                },
                _ = ev.closed() => break,
            }
        }
        let _ = p2.disconnect().await;
    });
    tokio::spawn(async move {
        while let Some(d) = txr.recv().await {
            let chunk = (p.mtu() as usize).saturating_sub(3).max(20);
            for c in d.chunks(chunk) {
                if let Err(x) = p.write(&ch, c, write_type).await {
                    let _ = ev2.send(LinkEvent::Closed(CloseReason::Detached(x.to_string()))).await;
                    return;
                }
            }
        }
    });
    let description = if dev.serial.is_empty() { format!("BLE {}", dev.name) } else { format!("BLE {} {}", dev.name, dev.serial) };
    Ok(Link { spec, description, tx, rx: evr })
}
