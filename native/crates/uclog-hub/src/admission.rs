//! Who may connect (hub plan §4.3, D3, D14).
//!
//! - Loopback peers are accepted, unless they are a browser WebSocket whose `Origin` is not on the
//!   allow-list (that stops arbitrary web pages from silently driving the device).
//! - Other peers are accepted with `--allow-all`, a matching `--allow-from CIDR`, or an entry in
//!   the saved allow-list (`hub.allow`). Otherwise the operator is asked on the console.

use std::collections::HashSet;
use std::io::Write as _;
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::AsyncBufReadExt;
use tokio::sync::{mpsc, oneshot};

/// Default `--allow-origin` patterns.
pub const DEFAULT_ORIGINS: &[&str] =
    &["http://localhost:*", "http://127.0.0.1:*", "http://[::1]:*", "https://localhost:*", "vscode-webview://*"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    addr: IpAddr,
    bits: u8,
}

impl FromStr for Cidr {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (a, b) = s.split_once('/').unwrap_or((s, ""));
        let addr: IpAddr = a.parse().map_err(|_| format!("bad address in {s:?}"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let bits = if b.is_empty() { max } else { b.parse().map_err(|_| format!("bad prefix length in {s:?}"))? };
        if bits > max {
            return Err(format!("prefix length too long in {s:?}"));
        }
        Ok(Cidr { addr, bits })
    }
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

impl Cidr {
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, canonical(ip)) {
            (IpAddr::V4(n), IpAddr::V4(a)) => {
                let m = if self.bits == 0 { 0 } else { u32::MAX << (32 - self.bits) };
                u32::from(n) & m == u32::from(a) & m
            }
            (IpAddr::V6(n), IpAddr::V6(a)) => {
                let m = if self.bits == 0 { 0 } else { u128::MAX << (128 - self.bits) };
                u128::from(n) & m == u128::from(a) & m
            }
            _ => false,
        }
    }
}

/// `*` matches any run of characters. A pattern ending in `:*` also matches without a port.
pub fn glob(pattern: &str, s: &str) -> bool {
    fn m(p: &[u8], s: &[u8]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((b'*', rest)) => (0..=s.len()).any(|i| m(rest, &s[i..])),
            Some((c, rest)) => s.first() == Some(c) && m(rest, &s[1..]),
        }
    }
    m(pattern.as_bytes(), s.as_bytes()) || pattern.strip_suffix(":*").is_some_and(|p| m(p.as_bytes(), s.as_bytes()))
}

pub fn is_loopback(ip: IpAddr) -> bool {
    canonical(ip).is_loopback()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Accept,
    /// Ask the operator; the text says why.
    Ask(String),
}

pub struct Rules {
    pub allow_all: bool,
    pub allow_from: Vec<Cidr>,
    pub allow_origin: Vec<String>,
    allow_file: Option<PathBuf>,
    saved: Mutex<HashSet<IpAddr>>,
}

impl Rules {
    pub fn new(allow_all: bool, allow_from: Vec<Cidr>, allow_origin: Vec<String>, allow_file: Option<PathBuf>) -> Self {
        let saved = allow_file
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| {
                t.lines()
                    .map(|l| l.split('#').next().unwrap_or("").trim())
                    .filter_map(|l| l.parse().ok())
                    .collect::<HashSet<IpAddr>>()
            })
            .unwrap_or_default();
        let allow_origin = if allow_origin.is_empty() {
            DEFAULT_ORIGINS.iter().map(|s| s.to_string()).collect()
        } else {
            allow_origin
        };
        Rules { allow_all, allow_from, allow_origin, allow_file, saved: Mutex::new(saved) }
    }

    /// `origin`: the WebSocket `Origin` header (browsers always send one; other clients do not).
    pub fn decide(&self, ip: IpAddr, origin: Option<&str>) -> Decision {
        let ip = canonical(ip);
        let origin_ok = origin.is_none_or(|o| self.allow_origin.iter().any(|p| glob(p, o)));
        if is_loopback(ip) && origin_ok {
            return Decision::Accept;
        }
        if self.allow_all || self.allow_from.iter().any(|c| c.contains(ip)) || self.saved.lock().unwrap().contains(&ip)
        {
            return Decision::Accept;
        }
        if !origin_ok {
            Decision::Ask(format!("browser page {} is not on the --allow-origin list", origin.unwrap_or("")))
        } else {
            Decision::Ask("remote host".into())
        }
    }

    /// "Always this host": remember it, in memory and in `hub.allow`.
    pub fn remember(&self, ip: IpAddr) -> std::io::Result<()> {
        let ip = canonical(ip);
        self.saved.lock().unwrap().insert(ip);
        if let Some(p) = &self.allow_file {
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d)?;
            }
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
            writeln!(f, "{ip}")?;
        }
        Ok(())
    }
}

/// Operator answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Always,
}

struct Question {
    text: String,
    reply: oneshot::Sender<Answer>,
}

/// Asks questions on the console; also watches stdin for EOF (`--exit-on-stdin-eof`).
#[derive(Clone)]
pub struct Prompter {
    tx: Option<mpsc::Sender<Question>>,
}

impl Prompter {
    /// `interactive`: stdin is a terminal. `on_eof` is signalled when stdin closes.
    pub fn start(interactive: bool, watch_eof: bool, on_eof: Option<oneshot::Sender<()>>) -> Self {
        if !interactive && !watch_eof {
            return Prompter { tx: None };
        }
        let (tx, mut rx) = mpsc::channel::<Question>(1);
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
            let mut on_eof = on_eof;
            let mut pending: Option<Question> = None;
            // Without a console no questions are asked (the sender is dropped), but stdin is
            // still watched for EOF.
            let mut asking = true;
            loop {
                tokio::select! {
                    q = rx.recv(), if pending.is_none() && asking => match q {
                        Some(q) => {
                            eprint!("{} [y]es / [n]o / [a]lways this host (60 s) ", q.text);
                            let _ = std::io::stderr().flush();
                            pending = Some(q);
                        }
                        None if watch_eof => asking = false,
                        None => return,
                    },
                    l = lines.next_line() => match l {
                        Ok(Some(l)) => if let Some(q) = pending.take() {
                            let a = match l.trim().to_ascii_lowercase().as_str() {
                                "y" | "yes" => Answer::Yes,
                                "a" | "always" => Answer::Always,
                                _ => Answer::No,
                            };
                            let _ = q.reply.send(a);
                        },
                        _ => {
                            if let Some(q) = pending.take() { let _ = q.reply.send(Answer::No); }
                            if let Some(s) = on_eof.take() { let _ = s.send(()); }
                            // stdin is gone: refuse any further questions.
                            while let Some(q) = rx.recv().await { let _ = q.reply.send(Answer::No); }
                            return;
                        }
                    },
                }
            }
        });
        Prompter { tx: interactive.then_some(tx) }
    }

    /// Ask; `No` on timeout or when there is no console.
    pub async fn ask(&self, text: String, timeout: Duration) -> Answer {
        let Some(tx) = &self.tx else { return Answer::No };
        let (rtx, rrx) = oneshot::channel();
        if tx.send(Question { text, reply: rtx }).await.is_err() {
            return Answer::No;
        }
        match tokio::time::timeout(timeout, rrx).await {
            Ok(Ok(a)) => a,
            _ => {
                eprintln!("\nno answer: rejected");
                Answer::No
            }
        }
    }

    pub fn interactive(&self) -> bool {
        self.tx.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn cidr() {
        let c: Cidr = "192.168.1.0/24".parse().unwrap();
        assert!(c.contains(ip("192.168.1.77")));
        assert!(c.contains(ip("::ffff:192.168.1.5")));
        assert!(!c.contains(ip("192.168.2.1")));
        let one: Cidr = "10.0.0.5".parse().unwrap();
        assert!(one.contains(ip("10.0.0.5")) && !one.contains(ip("10.0.0.6")));
        let v6: Cidr = "fe80::/10".parse().unwrap();
        assert!(v6.contains(ip("fe80::1")) && !v6.contains(ip("2001:db8::1")));
        assert!("0.0.0.0/0".parse::<Cidr>().unwrap().contains(ip("8.8.8.8")));
        assert!("1.2.3.4/33".parse::<Cidr>().is_err());
    }

    #[test]
    fn globs() {
        assert!(glob("http://localhost:*", "http://localhost:5173"));
        assert!(glob("http://localhost:*", "http://localhost"));
        assert!(!glob("http://localhost:*", "http://localhost.evil.com"));
        assert!(glob("vscode-webview://*", "vscode-webview://abc123"));
        assert!(!glob("http://127.0.0.1:*", "https://127.0.0.1:1"));
    }

    #[test]
    fn decisions() {
        let r = Rules::new(false, vec!["10.1.0.0/16".parse().unwrap()], vec![], None);
        assert_eq!(r.decide(ip("127.0.0.1"), None), Decision::Accept);
        assert_eq!(r.decide(ip("::1"), Some("http://localhost:8080")), Decision::Accept);
        assert!(matches!(r.decide(ip("127.0.0.1"), Some("https://evil.example")), Decision::Ask(_)));
        assert_eq!(r.decide(ip("10.1.2.3"), None), Decision::Accept);
        assert!(matches!(r.decide(ip("10.2.0.1"), None), Decision::Ask(_)));
        let all = Rules::new(true, vec![], vec![], None);
        assert_eq!(all.decide(ip("8.8.8.8"), None), Decision::Accept);
    }

    #[test]
    fn remember_persists() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("cfg/hub.allow");
        let r = Rules::new(false, vec![], vec![], Some(f.clone()));
        assert!(matches!(r.decide(ip("192.0.2.9"), None), Decision::Ask(_)));
        r.remember(ip("192.0.2.9")).unwrap();
        assert_eq!(r.decide(ip("192.0.2.9"), None), Decision::Accept);
        let r2 = Rules::new(false, vec![], vec![], Some(f));
        assert_eq!(r2.decide(ip("192.0.2.9"), None), Decision::Accept);
    }
}
