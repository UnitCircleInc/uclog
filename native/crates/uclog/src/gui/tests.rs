//! GUI tests: the real app, headless (egui_kittest), with the real backend and session against the
//! simulated `loopback:` device (GUI plan §7, task 12).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use uclog_proto::ports;

use super::app::{GuiApp, Tab};
use super::backend::{self, Cmd};
use super::plots::Kind;
use crate::settings::Settings;
use crate::{DeviceArgs, LogdataArgs};

fn sample() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../spec/vectors/sample.logdata")
}

struct Rig {
    h: Harness<'static, GuiApp>,
    _rt: tokio::runtime::Runtime,
}

fn rig(device: &str, logdata: bool, settings: Settings) -> Rig {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _g = rt.enter();
    let (backend, rx) = backend::start(
        egui::Context::default(),
        DeviceArgs { device: device.parse().unwrap(), baud: 1_000_000, usb_filter: vec![] },
        LogdataArgs { logdata: if logdata { vec![sample()] } else { vec![] }, ..Default::default() },
        None,
        settings.clone(),
    );
    let app = GuiApp::new(settings, None, backend, rx, None);
    let h = Harness::builder()
        .with_size(egui::vec2(1300.0, 850.0))
        .build_ui_state(|ui, app: &mut GuiApp| app.show(ui), app);
    Rig { h, _rt: rt }
}

impl Rig {
    /// Run frames until `f` holds (5 s at most).
    fn until(&mut self, what: &str, f: impl Fn(&GuiApp) -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !f(self.h.state()) {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            self.h.step();
            std::thread::sleep(Duration::from_millis(10));
        }
        self.h.step();
    }
}

#[test]
fn logs_terminal_device_and_status() {
    let mut s = Settings::default();
    s.terminal.on_connect = vec!["help".into()];
    let mut r = rig("loopback:rate=50", true, s);
    r.until("logs", |a| a.logs.received >= 10);
    r.until("help reply (on-connect line)", |a| a.term.text().contains("loopback commands"));
    let app = r.h.state();
    assert_eq!(app.logs.errors(), 0);
    assert!(app.logs.visible(0).unwrap().text.starts_with("Hello World!"));
    assert_eq!(app.device.app_hash.as_deref(), Some(&[0x5A; 64][..]));
    assert!(r.h.query_by_label_contains("connected: loopback").is_some(), "status bar");
    assert!(r.h.query_by_label_contains("logdata: manual").is_some());

    // Typing in the terminal sends a line.
    r.h.state_mut().focus(Tab::Terminal);
    r.h.step();
    r.h.ctx.memory_mut(|m| m.request_focus(egui::Id::new("term-input")));
    r.h.step();
    r.h.event(egui::Event::Text("echo from the gui".into()));
    r.h.step();
    r.h.key_press(egui::Key::Enter);
    r.until("echo", |a| a.term.text().contains("from the gui\n") || a.term.text().ends_with("from the gui"));
    assert_eq!(r.h.state().settings.terminal.history.last().map(String::as_str), Some("echo from the gui"));

    // Level filter: WARN hides the INFO lines.
    let a = r.h.state_mut();
    a.log_view.level = 2;
    a.set_filter();
    assert!((0..a.logs.visible_len()).all(|i| a.logs.visible(i).unwrap().level >= 2));
    // Search with a bad regex keeps the old filter and reports it.
    a.log_view.search = "(".into();
    a.set_filter();
    assert!(a.log_view.search_error.is_some());
}

#[test]
fn terminal_character_mode() {
    let mut r = rig("loopback:rate=0", false, Settings::default());
    r.until("connected", |a| a.conn.uri.is_some());
    r.h.state_mut().focus(Tab::Terminal);
    r.h.state_mut().term_view.char_mode = true;
    r.h.step();
    r.h.ctx.memory_mut(|m| m.request_focus(egui::Id::new("term-chars")));
    r.h.step();
    // Each key goes at once: no line is kept, and nothing reaches the history.
    for c in "help".chars() {
        r.h.event(egui::Event::Text(c.to_string()));
        r.h.step();
    }
    r.h.key_press(egui::Key::Enter);
    r.until("help reply", |a| a.term.text().contains("loopback commands"));
    assert!(r.h.state().term_view.char_focus);
    assert!(r.h.state().settings.terminal.history.is_empty());
}

#[test]
fn character_mode_keys() {
    use super::views::term::event_bytes;
    use egui::{Event, Key, Modifiers};
    let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    assert_eq!(event_bytes(&Event::Text("é".into())), Some("é".as_bytes().to_vec()));
    assert_eq!(event_bytes(&key(Key::Enter, Modifiers::NONE)), Some(b"\r".to_vec()));
    assert_eq!(event_bytes(&key(Key::ArrowUp, Modifiers::NONE)), Some(b"\x1b[A".to_vec()));
    assert_eq!(event_bytes(&key(Key::C, Modifiers::CTRL)), Some(vec![3]));
    assert_eq!(event_bytes(&key(Key::L, Modifiers::CTRL)), Some(vec![12]));
    assert_eq!(event_bytes(&key(Key::C, Modifiers::NONE)), None); // its Text event carries it
    assert_eq!(event_bytes(&key(Key::C, Modifiers::CTRL | Modifiers::ALT)), None); // AltGr
    assert_eq!(event_bytes(&Event::Paste("a\nb".into())), Some(b"a\rb".to_vec()));
    assert_eq!(event_bytes(&Event::Copy), (!cfg!(target_os = "macos")).then(|| vec![3]));
}

#[test]
fn plots_open_tabs_without_stealing_focus() {
    let mut r = rig("loopback:rate=0", false, Settings::default());
    r.until("connected", |a| a.conn.connected());
    r.h.state().send(ports::TERMINAL, b"plot\r".to_vec());
    r.until("plots", |a| a.plots.plots.len() == 3 && a.plots.frames > 20);
    let app = r.h.state();
    let dock = app.dock.as_ref().unwrap();
    for n in ["loopback", "spectrum", "grid"] {
        assert!(dock.find_tab(&Tab::Plot(n.into())).is_some(), "tab for {n}");
    }
    // The Logs view stays in front.
    let (_, leaf) = dock.iter_leaves().next().unwrap();
    assert_eq!(leaf.tabs[leaf.active.0], Tab::Logs);
    let Kind::Line(l) = &app.plots.get("loopback").unwrap().kind else { panic!() };
    assert_eq!(l.axes.len(), 2);
    assert!(l.axes[0].series[0].y.len() > 10);
    // Draw each plot (catches panics in the plot views).
    for n in ["loopback", "spectrum", "grid"] {
        r.h.state_mut().focus(Tab::Plot(n.into()));
        r.h.step();
        r.h.step();
    }
    r.h.state_mut().plot_views.get_mut("loopback").unwrap().cursors = Some([f64::NAN; 2]);
    r.h.state_mut().focus(Tab::Plot("loopback".into()));
    r.h.step();
    r.h.step();
    assert!(r.h.query_by_label_contains("Δx=").is_some(), "cursor readout");
    r.h.state().send(ports::TERMINAL, b"plot stop\r".to_vec());
}

#[test]
fn commands_buttons_and_free_line() {
    let mut s = Settings::default();
    s.commands.buttons = vec![
        crate::settings::Button { label: "Echo".into(), cmd: "echo button".into() },
        crate::settings::Button { label: "Arg".into(), cmd: serde_json::json!({"cmd": "x", "n": "$n"}) },
    ];
    let mut r = rig("loopback:rate=0", false, s);
    r.until("connected", |a| a.conn.connected());
    r.h.state_mut().focus(Tab::Commands);
    r.h.step();
    r.h.get_by_label("Echo").click();
    r.until("button reply on port 0", |a| a.cmds.responses.iter().any(|l| l == "button"));
    // The $n field is shown next to its button.
    assert!(r.h.query_by_label_contains("n $n").is_some());
    let a = r.h.state_mut();
    a.cmds.free = "echo free".into();
    r.h.step();
    r.h.get_by_label("Send").click();
    r.until("free line reply", |a| a.cmds.responses.iter().any(|l| l == "free"));
}

#[test]
fn connect_dialog_disconnect_and_hub_busy() {
    // A hub with a client already connected turns a second one away.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let hub = rt.block_on(async {
        use clap::Parser;
        #[derive(Parser)]
        struct T {
            #[command(flatten)]
            h: uclog_hub::HubArgs,
        }
        let a = T::parse_from([
            "t",
            "-d",
            "loopback:rate=5",
            "--port",
            "0",
            "--bind",
            "127.0.0.1",
            "--no-mdns",
            "--no-registry",
        ]);
        uclog_hub::start(a.h, 0).await.unwrap()
    });
    let uri = format!("tcp://127.0.0.1:{}", hub.port);
    let mut first = rig(&uri, true, Settings::default());
    first.until("first client connected", |a| a.logs.received > 0);
    let mut second = rig(&uri, true, Settings::default());
    second.until("busy", |a| a.conn.busy);
    assert!(second.h.query_by_label_contains("hub busy").is_some());
    // The first one disconnects; the Connect dialog connects the second.
    first.h.state().backend.send(Cmd::Disconnect);
    first.until("disconnected", |a| a.conn.closed.is_some());
    second.h.state_mut().open_connect();
    // Clicking a listed device fills in the address (the BLE list is only filled by a scan, so the
    // local device refresh can't replace it; the filter keeps a USB device arriving from shifting
    // the rows under the click).
    second.h.state_mut().connect.filter = "test hub".into();
    second.h.state_mut().connect.ble = vec![uclog_link::Device {
        kind: uclog_link::Kind::Hub,
        uri: uri.clone(),
        name: "test hub".into(),
        serial: String::new(),
        description: String::new(),
        busy: Some(false),
    }];
    second.h.step();
    second.h.get_by_label("test hub").click();
    second.h.step();
    assert_eq!(second.h.state().connect.address, uri);
    second.h.get_by_role_and_label(egui::accesskit::Role::Button, "Connect").click();
    second.until("second connected", |a| a.conn.connected() && a.logs.received > 0);
    assert!(!second.h.state().connect.open);
    rt.block_on(hub.shutdown());
}

#[test]
fn export_all_and_screenshot_names() {
    let d = tempfile::tempdir().unwrap();
    let s = Settings { export_dir: Some(d.path().into()), ..Default::default() };
    let mut r = rig("loopback:rate=50", true, s);
    r.until("logs", |a| a.logs.received >= 5);
    r.h.state().send(ports::TERMINAL, b"plot\r".to_vec());
    r.until("plots", |a| a.plots.plots.len() == 3);
    r.h.state_mut().export_all();
    let names: Vec<String> =
        std::fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names.len(), 5, "{names:?}");
    for part in ["-logs.csv", "-terminal.txt", "-plot-loopback.csv", "-plot-spectrum.csv", "-plot-grid.csv"] {
        assert!(names.iter().any(|n| n.ends_with(part)), "{part} in {names:?}");
    }
    r.h.state().send(ports::TERMINAL, b"plot stop\r".to_vec());
}

/// Hardware: `UCLOG_TEST_DEVICE=serial:/dev/… cargo test -p uclog hw_reconnect -- --ignored`.
#[test]
#[ignore]
fn hw_reconnect() {
    let dev = std::env::var("UCLOG_TEST_DEVICE").unwrap();
    let mut r = rig(&dev, false, Settings::default());
    r.until("first connection", |a| a.conn.connected() && a.logs.received > 0);
    r.h.state().backend.send(Cmd::Disconnect);
    r.until("disconnected", |a| a.conn.closed.is_some());
    // tokio retires idle blocking threads after 10 s: the device list below then runs on a new
    // thread (this crashed in hidapi on macOS).
    std::thread::sleep(Duration::from_secs(15));
    r.h.state_mut().open_connect();
    r.until("device list", |a| !a.connect.devices.is_empty());
    let n = r.h.state().logs.received;
    r.h.state().backend.send(Cmd::Connect(dev.parse().unwrap()));
    r.until("second connection", |a| a.conn.connected() && a.logs.received > n);
}

/// Screenshots for the docs (website/build.py screenshots), rendered with wgpu:
/// `UCLOG_SHOTS=dir cargo test -p uclog --bin uclog gui_screens -- --ignored`.
#[test]
#[ignore]
fn gui_screens() {
    let dir = PathBuf::from(std::env::var("UCLOG_SHOTS").expect("UCLOG_SHOTS: where to write the screenshots"));
    for theme in ["light", "dark"] {
        let mut s = Settings::default();
        s.gui.theme = theme.into();
        s.terminal.on_connect = vec!["help".into(), "plot".into()];
        let mut r = rig("loopback:rate=20", true, s);
        r.until("logs and plots", |a| a.logs.received >= 40 && a.plots.frames > 40);
        r.h.state_mut().focus(Tab::Logs);
        for _ in 0..3 {
            r.h.step();
        }
        r.h.render().unwrap().save(dir.join(format!("gui-logs-{theme}.png"))).unwrap();
        if theme == "light" {
            r.h.state_mut().focus(Tab::Plot("loopback".into()));
            for _ in 0..3 {
                r.h.step();
            }
            r.h.render().unwrap().save(dir.join("gui-plots-light.png")).unwrap();
        }
        r.h.state().send(ports::TERMINAL, b"plot stop\r".to_vec());
    }
}
