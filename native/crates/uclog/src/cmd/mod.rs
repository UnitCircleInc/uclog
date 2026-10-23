pub mod decode;
pub mod fw;
pub mod list;
pub mod term;

use anyhow::{anyhow, Context};
use uclog_link::{Link, LinkSpec};

use crate::DeviceArgs;

/// Resolve `auto` and open the link.
pub async fn open(d: &DeviceArgs) -> anyhow::Result<Link> {
    let spec = match &d.device {
        LinkSpec::Auto => uclog_link::discover::resolve_auto(&d.usb_filter).map_err(|(msg, list)| {
            let mut m = msg;
            for dev in list {
                m.push_str(&format!("\n  {:<40} {} {}", dev.uri, dev.name, dev.serial));
            }
            anyhow!(m)
        })?,
        s => s.clone(),
    }
    .with_baud(d.baud);
    let link = uclog_link::open(&spec).await.with_context(|| format!("opening {spec}"))?;
    eprintln!("uclog: connected to {}", link.description);
    Ok(link)
}

/// One-line rendering of a log record: `ts LEVEL file:line text`.
pub fn format_record(r: &uclog_session::LogRecord) -> String {
    let src = if r.line.is_empty() { r.file.clone() } else { format!("{}:{}", r.file, r.line) };
    format!("{:10.3} {:<5} {src} {}", r.ts, r.level, r.text)
}
