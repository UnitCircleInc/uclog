//! CLI tests for subcommands that need no device.

use std::process::Command;

use uclog_proto::frame::{wire, wire_port};

fn uclog() -> Command {
    Command::new(env!("CARGO_BIN_EXE_uclog"))
}

fn sample() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../spec/vectors/sample.logdata")
}

#[test]
fn decode_capture() {
    let d = tempfile::tempdir().unwrap();
    let mut cap = vec![];
    let mut log = 0xF010_0000u32.to_le_bytes().to_vec();
    log.extend_from_slice(b"capture\0");
    cap.extend(wire(&log));
    cap.extend(wire_port(61, b"terminal"));
    cap.extend(wire(&0xF020_0000u32.to_le_bytes()));
    let f = d.path().join("cap.bin");
    std::fs::write(&f, &cap).unwrap();

    let out = uclog().args(["decode", "-l"]).arg(sample()).arg("--all").arg(&f).output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("INFO  src/main.c:10 Hello World! capture"), "{stdout}");
    assert!(stdout.contains("port 61: \"terminal\""), "{stdout}");
    assert!(stdout.contains("no logdata for target 2"), "{stdout}");
    assert_eq!(out.status.code(), Some(2), "decode errors give exit status 2");
}

#[test]
fn bad_device_uri() {
    let out = uclog().args(["term", "-d", "bogus:x"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown scheme"));
}

/// `--exit-on-stdin-eof` with stdin a pipe (how scripts start a hub): the hub keeps running
/// while the pipe is open and exits cleanly when it closes, also when stderr is gone too (the
/// script was killed).
#[test]
fn hub_exits_on_stdin_eof_only() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let mut child = uclog()
        .args(["hub", "-d", "loopback:", "--port", "0", "--bind", "127.0.0.1", "--no-mdns"])
        .args(["--json-ready", "--exit-on-stdin-eof"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    assert!(line.contains("\"port\""), "{line}");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(child.try_wait().unwrap().is_none(), "hub exited with stdin still open");
    drop(child.stderr.take());
    drop(child.stdin.take());
    let t = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "hub did not exit on stdin EOF");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(status.success(), "{status:?}");
}
