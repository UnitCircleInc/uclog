//! `uclog gui` (docs/plans/04-native-gui.md): the egui/eframe GUI. The UI runs on the main thread;
//! the session runs on the tokio runtime (`backend`).

pub mod app;
pub mod backend;
pub mod connect;
pub mod plots;
pub mod settings_dlg;
pub mod term;
pub mod views;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::Args;

use self::app::{GuiApp, Tab};
use crate::settings::Settings;
use crate::{DeviceArgs, LogdataArgs};

#[derive(Args)]
pub struct GuiArgs {
    #[command(flatten)]
    device: DeviceArgs,
    #[command(flatten)]
    logdata: LogdataArgs,
    /// Semihost sandbox directory [default: from settings]
    #[arg(long, value_name = "DIR")]
    files_dir: Option<PathBuf>,
    /// Settings file [default: the platform config directory]
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Save a screenshot after --screenshot-after seconds, then exit (docs, tests)
    #[arg(long, value_name = "PNG", hide = true)]
    screenshot: Option<PathBuf>,
    #[arg(long, value_name = "SECS", default_value_t = 3.0, hide = true)]
    screenshot_after: f64,
    /// Lines sent on port 61 after connecting (in addition to the settings'), e.g. for screenshots
    #[arg(long = "send", value_name = "LINE", hide = true)]
    send: Vec<String>,
    /// Views to bring to the front before the screenshot (Logs, Terminal, Commands, Device, FW, or a plot name)
    #[arg(long, value_name = "VIEW", hide = true)]
    show: Vec<String>,
    /// Initial window size, WxH
    #[arg(long, value_name = "WxH", hide = true)]
    size: Option<String>,
}

struct Eframe {
    app: GuiApp,
}

impl eframe::App for Eframe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.show(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(d) = &self.app.dock {
            eframe::set_value(storage, "dock", d);
        }
        self.app.save_settings();
    }

    /// Only the dock layout and the window are kept: egui's own memory (scroll offsets, popups)
    /// is not worth restoring, and after an egui upgrade it fails to parse ("Failed to parse RON").
    fn persist_egui_memory(&self) -> bool {
        false
    }

    /// eframe has saved by now (`save` runs first). On macOS, exit here rather than let winit tear
    /// the window down: AppKit can throw from its Touch Bar observer (`NSRangeException`, "Cannot
    /// remove an observer … for the key path nextResponder") and abort the process.
    #[cfg(target_os = "macos")]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        std::process::exit(0);
    }
}

/// Runs on the main thread, inside the tokio runtime's context (`Runtime::enter`).
pub fn run(a: GuiArgs) -> anyhow::Result<ExitCode> {
    let (mut settings, settings_path, note) = Settings::load(a.config.as_deref());
    settings.terminal.on_connect.extend(a.send.iter().cloned());
    let size = a
        .size
        .as_deref()
        .and_then(|s| s.split_once('x'))
        .and_then(|(w, h)| Some([w.parse::<f32>().ok()?, h.parse::<f32>().ok()?]))
        .unwrap_or([1280.0, 820.0]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ucLog")
            .with_app_id("ca.unitcircle.uclog")
            .with_inner_size(size)
            .with_min_inner_size([640.0, 400.0]),
        // A screenshot run must not change the user's saved layout.
        persist_window: a.screenshot.is_none(),
        ..Default::default()
    };
    let shot = a.screenshot.clone().map(|p| (p, a.screenshot_after));
    eframe::run_native(
        "ucLog",
        options,
        Box::new(move |cc| {
            let dock = if shot.is_none() {
                cc.storage.and_then(|s| eframe::get_value::<egui_dock::DockState<Tab>>(s, "dock"))
            } else {
                None
            };
            let from_keyring = crate::front::token_from_keyring(&a.logdata);
            let (backend, rx) = backend::start(cc.egui_ctx.clone(), a.device, a.logdata, a.files_dir, settings.clone());
            let mut app = GuiApp::new(settings, settings_path, backend, rx, dock);
            app.token_from_keyring = from_keyring;
            if let Some(n) = note {
                app.say(n);
            }
            if let Some((p, after)) = shot {
                app.screenshot(Some(p));
                app.script.show = a.show.clone();
                app.script.at = Some(Instant::now() + Duration::from_secs_f64(after));
                app.script.exit = true;
                // Don't touch the user's settings file from a screenshot run.
                app.settings_path = None;
            }
            Ok(Box::new(Eframe { app }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(ExitCode::SUCCESS)
}
