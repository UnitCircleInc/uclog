//! `uclog decode`: decode a raw wire capture (replaces logfile.py).

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;
use clap::Args;
use uclog_logdata::{Decoders, LogData};
use uclog_proto::{cbor, cobs, mux_decode, ports, FrameScanner, Mux, ScanEvent};

use crate::LogdataArgs;

#[derive(Args)]
pub struct DecodeArgs {
    #[command(flatten)]
    logdata: LogdataArgs,
    /// Also print non-log frames (ports) as hex/CBOR
    #[arg(long)]
    all: bool,
    /// Raw capture file (the bytes received from the device); `-` for stdin
    file: PathBuf,
}

pub async fn run(a: DecodeArgs) -> anyhow::Result<ExitCode> {
    let mut data = vec![];
    if a.file.as_os_str() == "-" {
        std::io::stdin().read_to_end(&mut data)?;
    } else {
        data = std::fs::read(&a.file).with_context(|| a.file.display().to_string())?;
    }
    let mut dec = Decoders::default();
    for p in &a.logdata.logdata {
        dec.insert(Arc::new(LogData::load(p).with_context(|| p.display().to_string())?));
    }
    let resolver = a.logdata.resolver();
    let mut errors = 0usize;
    let mut frames = vec![];
    // A capture may start mid-frame; treat the start of the file as a delimiter.
    let mut sc = FrameScanner::default();
    sc.feed(&[0]);
    sc.feed_with(&data, |e| frames.push(e));
    sc.feed_with(&[0], |e| frames.push(e));
    for e in frames {
        let enc = match e {
            ScanEvent::Frame(f) => f,
            ScanEvent::Oversize(n) => {
                errors += 1;
                println!("ERROR <uclog> framing: frame exceeded {} bytes, discarded {n} bytes", uclog_proto::MAX_FRAME);
                continue;
            }
        };
        let Ok(p) = cobs::decode(&enc) else {
            errors += 1;
            println!("ERROR <uclog> framing: invalid COBS ({} bytes)", enc.len());
            continue;
        };
        match mux_decode(&p) {
            Mux::Pulse => {}
            Mux::ShortLog(_) => {
                errors += 1;
                println!("ERROR <uclog> decode: log frame too short");
            }
            Mux::Log { target, addr, args, .. } => match dec.get(target) {
                Some(ld) => match ld.decode(addr, args) {
                    Ok(d) => {
                        let src = if d.line.is_empty() { d.file } else { format!("{}:{}", d.file, d.line) };
                        println!("{:<5} {src} {}", d.level, d.text);
                    }
                    Err(e) => {
                        errors += 1;
                        println!("ERROR <uclog> decode: {e}");
                    }
                },
                None => {
                    errors += 1;
                    println!(
                        "ERROR <uclog> decode: no logdata for target {target} (addr 0x{addr:08x}) [{}]",
                        hex::encode(args)
                    );
                }
            },
            Mux::Port { port: ports::DEVICE_INFO, data } => {
                let Ok(info) = cbor::from_slice(data) else { continue };
                if a.all {
                    println!("port 62: {}", cbor::display(&info));
                }
                if a.logdata.logdata.is_empty() {
                    if let Some(h) = cbor::get_bytes(&info, "app_hash") {
                        let o = uclog_session::resolver::resolve(&resolver, h).await;
                        for n in o.notes {
                            eprintln!("uclog: {n}");
                        }
                        match o.resolved {
                            Some(r) => {
                                eprintln!("uclog: logdata for app_hash {}…: {}", &hex::encode(h)[..16], r.source);
                                dec.clear();
                                for ld in r.logdata {
                                    dec.insert(ld);
                                }
                            }
                            None => eprintln!("uclog: no logdata found for app_hash {}", hex::encode(h)),
                        }
                    }
                }
            }
            Mux::Port { port, data } if a.all => match cbor::from_slice(data) {
                Ok(v) if port != ports::TERMINAL => println!("port {port}: {}", cbor::display(&v)),
                _ => println!("port {port}: {:?}", String::from_utf8_lossy(data)),
            },
            Mux::Port { .. } => {}
        }
    }
    Ok(if errors > 0 { ExitCode::from(2) } else { ExitCode::SUCCESS })
}
