//! `uclog fw-update`

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::Args;
use uclog_session::{Event, FwProgress, Liveness};

use crate::{DeviceArgs, LogdataArgs};

#[derive(Args)]
pub struct FwArgs {
    #[command(flatten)]
    device: DeviceArgs,
    #[command(flatten)]
    logdata: LogdataArgs,
    /// Signed image (e.g. build/zephyr/zephyr.signed.bin)
    image: PathBuf,
}

pub async fn run(a: FwArgs) -> anyhow::Result<ExitCode> {
    let image = std::fs::read(&a.image).with_context(|| a.image.display().to_string())?;
    if let Some(h) = uclog_session::resolver::image_hash(&image) {
        eprintln!("uclog: image {} ({} bytes), app_hash {}…", a.image.display(), image.len(), &hex::encode(h)[..16]);
    }
    let link = super::open(&a.device).await?;
    let cfg = uclog_session::Config { resolver: a.logdata.resolver(), ..Default::default() };
    let (h, mut ev) = uclog_session::start(link, cfg);
    // Drain events in the background; report liveness and closure.
    let watcher = tokio::spawn(async move {
        while let Some(e) = ev.recv().await {
            match e {
                Event::Liveness(Liveness::NotResponding) => eprintln!("\nuclog: device not responding"),
                Event::Closed(r) => {
                    eprintln!("\nuclog: {r}");
                    return;
                }
                _ => {}
            }
        }
    });
    let r = h
        .fw_update(&image, |p| match p {
            FwProgress::Probing => eprint!("uclog: probing FW update port… "),
            FwProgress::Port(p) => eprintln!("port {p}{}", if p == 3 { " (legacy)" } else { "" }),
            FwProgress::Starting => eprintln!("uclog: erasing (fw-start)…"),
            FwProgress::Block { done, total } => {
                eprint!("\ruclog: writing block {done}/{total} ({}%)", done * 100 / total);
                let _ = std::io::stderr().flush();
            }
            FwProgress::Scheduling => eprintln!("\nuclog: scheduling update"),
            FwProgress::Done => {}
        })
        .await;
    h.add_image(a.image.clone()).await;
    h.close().await;
    let _ = watcher.await;
    match r {
        Ok(port) => {
            eprintln!("uclog: FW update scheduled (port {port}); the device restarts in about 5 s");
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => {
            eprintln!("\nuclog: FW update failed: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}
