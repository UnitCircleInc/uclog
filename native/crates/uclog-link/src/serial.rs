//! Serial (UART bridge / USB CDC-ACM) link (protocol.md §4.1).

use std::io::{ErrorKind, Read, Write};
use std::time::Duration;

use uclog_proto::link::{stlink_split, STLINK_SPLIT_GAP};

use crate::{channels, CloseReason, Link, LinkError, LinkEvent, LinkSpec};

fn is_stlink(path: &str) -> bool {
    serialport::available_ports().unwrap_or_default().into_iter().any(|p| {
        p.port_name == path && matches!(p.port_type, serialport::SerialPortType::UsbPort(ref u) if u.vid == 0x0483)
    })
}

pub(crate) fn open(spec: LinkSpec, path: &str, baud: u32) -> Result<Link, LinkError> {
    let mut port = serialport::new(path, baud).timeout(Duration::from_millis(50)).open()?;
    // On Unix, serialport opens the port with TIOCEXCL, so two processes cannot share it
    // (hub plan §3).
    let _ = port.write_data_terminal_ready(true);
    let mut writer = port.try_clone()?;
    let stlink = is_stlink(path);
    let (tx, mut txr, ev, evr) = channels();

    let ev2 = ev.clone();
    std::thread::Builder::new().name(format!("serial-rx {path}")).spawn(move || {
        let mut buf = [0u8; 4096];
        // A read that returns 0 after the port polled readable is end of file (the device went
        // away). Allow a few in case a driver does it spuriously, but never spin on it.
        let mut eofs = 0;
        loop {
            match port.read(&mut buf) {
                Ok(0) => {
                    eofs += 1;
                    if eofs >= 3 {
                        let _ = ev.blocking_send(LinkEvent::Closed(CloseReason::Detached("end of file".into())));
                        return;
                    }
                    if ev.is_closed() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(n) => {
                    eofs = 0;
                    if ev.blocking_send(LinkEvent::Data(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::Interrupted => {
                    eofs = 0;
                    if ev.is_closed() {
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
    std::thread::Builder::new().name(format!("serial-tx {path}")).spawn(move || {
        while let Some(data) = txr.blocking_recv() {
            let parts = if stlink { stlink_split(&data) } else { vec![&data[..]] };
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    let _ = writer.flush();
                    std::thread::sleep(STLINK_SPLIT_GAP);
                }
                if let Err(e) = writer.write_all(part) {
                    let _ = ev2.blocking_send(LinkEvent::Closed(CloseReason::Detached(e.to_string())));
                    return;
                }
            }
        }
    })?;
    Ok(Link { description: format!("serial {path}{}", if stlink { " (STLINK)" } else { "" }), spec, tx, rx: evr })
}
