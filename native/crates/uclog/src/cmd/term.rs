//! `uclog term`: minimal terminal on port 61 (or `--port 0` for text commands, like logcmd.py).

use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use crossterm::event::{Event as TEvent, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_lite::StreamExt;
use tokio::io::AsyncBufReadExt;
use uclog_session::{Event, Liveness};

use crate::{DeviceArgs, LogdataArgs};

#[derive(Args)]
pub struct TermArgs {
    #[command(flatten)]
    device: DeviceArgs,
    #[command(flatten)]
    logdata: LogdataArgs,
    /// Port to talk to (61 = terminal, 0 = text commands)
    #[arg(long, default_value_t = 61)]
    port: u8,
    /// Character mode: send each key as typed (Ctrl-] to quit)
    #[arg(long)]
    raw: bool,
    /// Also print decoded log lines
    #[arg(long)]
    logs: bool,
    /// Semihost sandbox directory (file access from the device)
    #[arg(long, value_name = "DIR")]
    files_dir: Option<std::path::PathBuf>,
    /// Send this line on connect (repeatable), e.g. -c "version s"
    #[arg(short = 'c', long = "command", value_name = "LINE")]
    commands: Vec<String>,
    /// With -c: exit this many seconds after sending instead of staying interactive
    #[arg(long, value_name = "SECS")]
    exit_after: Option<f64>,
}

struct RawGuard;

impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn key_bytes(k: KeyEvent) -> Option<Vec<u8>> {
    if k.kind == KeyEventKind::Release {
        return None;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    Some(match k.code {
        KeyCode::Char(c) if ctrl && c.is_ascii_alphabetic() => vec![(c.to_ascii_lowercase() as u8) - b'a' + 1],
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Backspace => vec![0x7F],
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::Esc => vec![0x1B],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => return None,
    })
}

pub async fn run(a: TermArgs) -> anyhow::Result<ExitCode> {
    let link = super::open(&a.device).await?;
    let cfg = uclog_session::Config {
        resolver: a.logdata.resolver(),
        semihost: uclog_session::SemihostConfig { files_dir: a.files_dir.clone(), ..Default::default() },
        ..Default::default()
    };
    let (h, mut ev) = uclog_session::start(link, cfg);
    for c in &a.commands {
        h.send(a.port, format!("{c}\r").into_bytes()).await?;
    }
    let deadline = a.exit_after.map(|s| tokio::time::Instant::now() + Duration::from_secs_f64(s));
    let _raw = if a.raw && deadline.is_none() {
        crossterm::terminal::enable_raw_mode()?;
        eprint!("uclog: character mode, Ctrl-] to quit\r\n");
        Some(RawGuard)
    } else {
        None
    };
    // In raw mode the terminal needs \r\n.
    let nl = if a.raw { "\r\n" } else { "\n" };
    // Only in raw mode: EventStream needs a TTY.
    let raw_keys = a.raw && deadline.is_none();
    let mut keys = raw_keys.then(EventStream::new);
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut stdin_wanted = false;
    let mut stdin_open = deadline.is_none();
    let mut out = std::io::stdout();
    loop {
        tokio::select! {
            _ = async { tokio::time::sleep_until(deadline.unwrap()).await }, if deadline.is_some() => break,
            e = ev.recv() => {
                let Some(e) = e else { break };
                match e {
                    Event::Port { port, data } if port == a.port => {
                        let text = String::from_utf8_lossy(&data);
                        let text = if a.raw { text.replace('\n', "\r\n").replace("\r\r\n", "\r\n") } else { text.into_owned() };
                        out.write_all(text.as_bytes())?;
                        out.flush()?;
                    }
                    Event::Log(r) if a.logs || r.error.is_some() => {
                        if a.logs {
                            write!(out, "{}{nl}", super::format_record(&r))?;
                        }
                    }
                    Event::Console(c) => {
                        out.write_all(&c.data)?;
                        out.flush()?;
                    }
                    Event::StdinWanted => {
                        stdin_wanted = true;
                        eprint!("{nl}uclog: device is reading stdin; type a line{nl}");
                    }
                    Event::Liveness(Liveness::NotResponding) => eprint!("{nl}uclog: device not responding{nl}"),
                    Event::Liveness(Liveness::Alive) => {}
                    Event::Liveness(Liveness::Waiting) => {
                        if a.device.device.is_hub() {
                            eprint!("uclog: connected; waiting for data (a remote hub may be waiting for operator approval){nl}");
                        }
                    }
                    Event::Note(n) => eprint!("uclog: {n}{nl}"),
                    Event::Logdata { source: Some(s), .. } => eprint!("uclog: logdata: {s}{nl}"),
                    Event::Closed(r) => {
                        eprint!("{nl}uclog: {r}{nl}");
                        return Ok(ExitCode::FAILURE);
                    }
                    _ => {}
                }
            }
            k = async { keys.as_mut().unwrap().next().await }, if raw_keys => {
                let Some(Ok(TEvent::Key(k))) = k else { continue };
                if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char(']') {
                    break;
                }
                if let Some(b) = key_bytes(k) {
                    if stdin_wanted {
                        stdin_wanted = false;
                        h.stdin(b).await?;
                    } else {
                        h.send(a.port, b).await?;
                    }
                }
            }
            l = lines.next_line(), if !a.raw && stdin_open => {
                match l? {
                    Some(l) => {
                        if stdin_wanted {
                            stdin_wanted = false;
                            h.stdin(format!("{l}\n").into_bytes()).await?;
                        } else {
                            h.send(a.port, format!("{l}\r").into_bytes()).await?;
                        }
                    }
                    None => stdin_open = false,
                }
            }
        }
    }
    h.close().await;
    Ok(ExitCode::SUCCESS)
}
