//! UI tests: the real App and drawing code against the loopback device, rendered into
//! ratatui's TestBackend (TUI plan §6, task 10).

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use uclog_proto::cbor::{self, Value};

use super::app::{button_payload, Action, App, Bg, Modal, Tab};
use super::ui;
use crate::settings::{Button, Settings};

fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn screen(app: &App) -> String {
    let mut t = Terminal::new(TestBackend::new(140, 40)).unwrap();
    t.draw(|f| ui::draw(f, app)).unwrap();
    let b = t.backend().buffer().clone();
    (0..b.area.height)
        .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect::<String>() + "\n")
        .collect()
}

fn sample_logdata() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../spec/vectors/sample.logdata")
}

/// Run a loopback session for `secs`, feeding everything to the app.
async fn run_loopback(app: &mut App, opts: &str, secs: f64, send: &[(u8, &[u8])]) {
    let spec: uclog_link::LinkSpec = format!("loopback:{opts}").parse().unwrap();
    let link = uclog_link::open(&spec).await.unwrap();
    let description = link.description.clone();
    let mut cfg = uclog_session::Config::default();
    cfg.resolver.manual = vec![sample_logdata()];
    let (h, mut ev) = uclog_session::start(link, cfg);
    app.on_bg(Bg::Connected { uri: spec.to_string(), description });
    for (p, d) in send {
        h.send(*p, d.to_vec()).await.unwrap();
    }
    let end = tokio::time::Instant::now() + Duration::from_secs_f64(secs);
    while let Ok(Some(e)) = tokio::time::timeout_at(end, ev.recv()).await {
        app.on_session(e);
    }
    h.close().await;
}

#[tokio::test]
async fn logs_view_with_loopback_device() {
    let mut app = App::new(Settings::default(), PathBuf::from("."));
    run_loopback(&mut app, "rate=20", 1.2, &[(61, b"echo hello tui\r")]).await;
    let s = screen(&app);
    assert!(s.contains("● live"), "{s}");
    assert!(s.contains("loopback (loopback)"), "{s}");
    assert!(s.contains("Hello World! loopback start"), "{s}");
    assert!(s.contains("temp="), "{s}");
    assert!(s.contains("src/main.c:10"), "{s}");
    assert!(s.contains("hello tui"), "terminal pane shows port 61: {s}");
    assert!(s.contains("errors: 0"), "{s}");
    // Device view: device info and logdata source.
    app.tab = Tab::Device;
    let s = screen(&app);
    assert!(s.contains("app_hash") && s.contains("5a5a5a5a"), "{s}");
    assert!(s.contains("board") && s.contains("loopback"), "{s}");
}

#[tokio::test]
async fn level_filter_and_search() {
    let mut app = App::new(Settings::default(), PathBuf::from("."));
    run_loopback(&mut app, "rate=30", 1.0, &[]).await;
    let all = app.logs.visible_len();
    assert!(all >= 10);
    // "no args" is INFO, "temp=" is WARN, "Hello" is INFO in sample.logdata: + twice → WARN and up.
    app.on_key(key(KeyCode::Char('+')));
    app.on_key(key(KeyCode::Char('+')));
    let s = screen(&app);
    assert!(s.contains("filter: level ≥ WARN"), "{s}");
    assert!(!s.contains("no args"), "{s}");
    assert!(s.contains("temp="), "{s}");
    app.on_key(key(KeyCode::Char('x')));
    assert_eq!(app.logs.visible_len(), all);
    app.on_key(key(KeyCode::Char('/')));
    for c in "Hello".chars() {
        app.on_key(key(KeyCode::Char(c)));
    }
    app.on_key(key(KeyCode::Enter));
    let n = app.logs.visible_len();
    assert!(n >= 1 && n < all);
    assert!((0..n).all(|i| app.logs.visible(i).unwrap().text.contains("Hello")));
}

#[tokio::test]
async fn error_records_are_counted_without_logdata() {
    let mut app = App::new(Settings::default(), PathBuf::from("."));
    let spec: uclog_link::LinkSpec = "loopback:rate=20".parse().unwrap();
    let link = uclog_link::open(&spec).await.unwrap();
    let (h, mut ev) = uclog_session::start(link, uclog_session::Config::default());
    let end = tokio::time::Instant::now() + Duration::from_millis(600);
    while let Ok(Some(e)) = tokio::time::timeout_at(end, ev.recv()).await {
        app.on_session(e);
    }
    h.close().await;
    assert!(app.logs.errors() > 0);
    let s = screen(&app);
    assert!(s.contains("decode: no logdata for target 1"), "{s}");
    assert!(!s.contains("errors: 0"), "{s}");
}

#[test]
fn terminal_line_and_character_modes() {
    let mut app = App::new(Settings::default(), PathBuf::from("."));
    app.on_key(key(KeyCode::Char('2')));
    assert_eq!(app.tab, Tab::Terminal);
    for c in "help".chars() {
        assert!(app.on_key(key(KeyCode::Char(c))).is_empty());
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), vec![Action::Send(61, b"help\r".to_vec())]);
    assert_eq!(app.settings.terminal.history.last().unwrap(), "help");
    // 'q' is typed, not quit, in the terminal.
    assert!(app.on_key(key(KeyCode::Char('q'))).is_empty());
    app.on_key(key(KeyCode::Backspace));
    // Character mode: keys go straight to the device; Ctrl+C too.
    app.on_key(ctrl('t'));
    assert_eq!(app.on_key(key(KeyCode::Char('x'))), vec![Action::Send(61, b"x".to_vec())]);
    assert_eq!(app.on_key(key(KeyCode::Up)), vec![Action::Send(61, b"\x1b[A".to_vec())]);
    assert_eq!(app.on_key(ctrl('c')), vec![Action::Send(61, vec![3])]);
    assert_eq!(app.on_key(ctrl('q')), vec![Action::Quit]);
    // Alt+4 switches views even while typing; Esc in the Terminal view goes back to Logs.
    app.on_key(ctrl('t'));
    app.on_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::ALT));
    assert_eq!(app.tab, Tab::Device);
    app.on_key(key(KeyCode::Char('2')));
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.tab, Tab::Logs);
    // F6 = first terminal button.
    assert_eq!(app.on_key(key(KeyCode::F(6))), vec![Action::Send(61, b"help\r".to_vec())]);
    // Output with ANSI colour renders as text.
    app.term.append(b"\x1b[32mgreen ok\x1b[0m\r\n");
    assert!(screen(&app).contains("green ok"));
}

#[test]
fn command_buttons_with_args() {
    let mut s = Settings::default();
    s.commands.buttons = vec![
        Button { label: "Ping".into(), cmd: serde_json::json!({"f": "cmd_ping"}) },
        Button { label: "Set".into(), cmd: serde_json::json!({"cmd": "set", "value": "$v", "name": "x$a-$b"}) },
        Button { label: "Text".into(), cmd: serde_json::json!("version s") },
    ];
    let mut app = App::new(s, PathBuf::from("."));
    app.on_key(key(KeyCode::Char('3')));
    let a = app.on_key(key(KeyCode::Enter));
    let Action::Send(0, d) = &a[0] else { panic!("{a:?}") };
    assert_eq!(cbor::from_slice(d).unwrap(), cbor::map([("f", Value::Text("cmd_ping".into()))]));
    // Second button: three $args are asked, in the order written in the settings.
    app.on_key(key(KeyCode::Down));
    assert!(app.on_key(key(KeyCode::Enter)).is_empty());
    let mut out = vec![];
    for answer in ["42", "A", "B"] {
        for c in answer.chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        out = app.on_key(key(KeyCode::Enter));
    }
    let Action::Send(0, d) = &out[0] else { panic!("{out:?}") };
    let v = cbor::from_slice(d).unwrap();
    assert_eq!(cbor::get_int(&v, "value"), Some(42), "a single $arg that parses becomes a number");
    assert_eq!(cbor::get_text(&v, "name"), Some("xA-B"));
    // Text button: a line.
    app.on_key(key(KeyCode::Down));
    assert_eq!(app.on_key(key(KeyCode::Enter)), vec![Action::Send(0, b"version s\r".to_vec())]);
    // CBOR responses are shown pretty-printed.
    app.on_session(uclog_session::Event::Port {
        port: 0,
        data: cbor::to_vec(&cbor::map([("rsp", Value::Text("set".into()))])),
    });
    assert!(screen(&app).contains("rsp"));
    let b = Button { label: "f".into(), cmd: serde_json::json!({"t": "$x"}) };
    let p = button_payload(&b, &[("t".into(), "$x".into(), "1.5".into())]);
    assert_eq!(cbor::from_slice(&p).unwrap(), cbor::map([("t", Value::Float(1.5))]));
}

#[test]
fn connect_dialog_and_settings() {
    let mut app = App::new(Settings::default(), PathBuf::from("."));
    assert_eq!(app.on_key(key(KeyCode::F(2))), vec![Action::RefreshDevices]);
    app.on_bg(Bg::Devices(vec![uclog_link::Device {
        kind: uclog_link::Kind::Hub,
        uri: "tcp://localhost:9000".into(),
        name: "lab-ttyACM0".into(),
        serial: "0042".into(),
        description: "serial (local hub)".into(),
        busy: Some(true),
    }]));
    let s = screen(&app);
    assert!(s.contains("tcp://localhost:9000") && s.contains("[busy]"), "{s}");
    let a = app.on_key(key(KeyCode::Enter));
    assert!(matches!(&a[0], Action::Connect(uclog_link::LinkSpec::Tcp { addr }) if addr == "localhost:9000"));
    // Add an address: a bare host means a hub.
    app.on_key(key(KeyCode::F(2)));
    app.on_key(key(KeyCode::Char('a')));
    for c in "lab-pc".chars() {
        app.on_key(key(KeyCode::Char(c)));
    }
    let a = app.on_key(key(KeyCode::Enter));
    assert!(matches!(&a[0], Action::Connect(uclog_link::LinkSpec::Tcp { addr }) if addr == "lab-pc:9000"), "{a:?}");
    // Settings: edit max lines and save.
    app.on_key(ctrl('s'));
    for _ in 0..6 {
        app.on_key(key(KeyCode::Down));
    }
    app.on_key(key(KeyCode::Enter));
    app.on_key(ctrl('u'));
    for c in "500".chars() {
        app.on_key(key(KeyCode::Char(c)));
    }
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.settings.max_log_lines, 500);
    assert!(screen(&app).contains("Max log lines"));
    assert_eq!(app.on_key(key(KeyCode::Char('s'))), vec![Action::SaveSettings { token: None }]);
}

#[test]
fn export_and_help() {
    let mut app = App::new(Settings::default(), PathBuf::from("."));
    let a = app.on_key(ctrl('e'));
    assert!(matches!(&a[0], Action::Export { name, content } if name == "logs.csv" && content.starts_with("ts,level")));
    app.on_key(key(KeyCode::F(1)));
    assert!(screen(&app).contains("Help (Esc)"));
    app.on_key(key(KeyCode::Esc));
    assert!(!screen(&app).contains("Help (Esc)"));
}

#[test]
fn refused_token_prompt() {
    let mut app = App::new(Settings::default(), PathBuf::from("."));
    app.token_set = true;
    app.on_session(uclog_session::Event::TokenRefused);
    assert!(matches!(&app.modal, Modal::Prompt(p) if p.title.contains("Remove the saved token?")));
    app.on_key(key(KeyCode::Char('y')));
    assert_eq!(app.on_key(key(KeyCode::Enter)), vec![Action::SaveSettings { token: Some(None) }]);
    assert!(!app.token_set);
    // Asked once per run.
    app.token_set = true;
    app.on_session(uclog_session::Event::TokenRefused);
    assert!(matches!(app.modal, Modal::None));

    // A token from the command line or the environment: advice only.
    let mut app = App::new(Settings::default(), PathBuf::from("."));
    app.token_set = true;
    app.token_from_keyring = false;
    app.on_session(uclog_session::Event::TokenRefused);
    assert!(matches!(app.modal, Modal::None));
    assert!(screen(&app).contains("UCLOG_LOGDATA_TOKEN"));
}
