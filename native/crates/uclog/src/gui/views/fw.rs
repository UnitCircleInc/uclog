//! FW update view: pick an image (`*.signed.bin` under the working directory are suggested), then
//! port 59 with fallback to 3, progress and cancel (GUI plan §3).

use std::path::PathBuf;

use egui::{Color32, RichText, Ui};

use crate::gui::app::GuiApp;
use crate::gui::backend::Cmd;

pub fn ui(app: &mut GuiApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label("Image");
        ui.add(
            egui::TextEdit::singleline(&mut app.fw.path)
                .desired_width(ui.available_width() - 90.0)
                .hint_text("zephyr.signed.bin"),
        );
        if ui.button("Browse…").clicked() {
            if let Some(p) = rfd::FileDialog::new()
                .set_title("FW image")
                .add_filter("Signed image", &["bin"])
                .add_filter("All files", &["*"])
                .set_directory(&app.cwd)
                .pick_file()
            {
                app.fw.path = p.display().to_string();
            }
        }
    });
    if !app.fw.suggestions.is_empty() {
        ui.label(RichText::new("Found under the working directory:").weak());
        for s in app.fw.suggestions.clone() {
            let rel = s.strip_prefix(&app.cwd).unwrap_or(&s).display().to_string();
            if ui.selectable_label(app.fw.path == s.display().to_string(), rel).clicked() {
                app.fw.path = s.display().to_string();
            }
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let can = app.conn.connected() && !app.fw.running && !app.fw.path.trim().is_empty();
        if ui.add_enabled(can, egui::Button::new("Start update")).clicked() {
            app.fw.running = true;
            app.fw.result = None;
            app.fw.progress = None;
            app.backend.send(Cmd::FwStart(PathBuf::from(app.fw.path.trim())));
        }
        if ui.add_enabled(app.fw.running, egui::Button::new("Cancel")).clicked() {
            app.backend.send(Cmd::FwCancel);
        }
        if !app.conn.connected() {
            ui.label(RichText::new("not connected").weak());
        }
    });
    if let Some(p) = &app.fw.progress {
        let (r, label) = crate::front::fw_ratio(p);
        ui.add(egui::ProgressBar::new(r as f32).text(label).animate(app.fw.running));
    }
    match &app.fw.result {
        Some(Ok(s)) => {
            ui.label(RichText::new(s).color(Color32::from_rgb(90, 190, 110)));
        }
        Some(Err(e)) => {
            ui.label(RichText::new(format!("Failed: {e}")).color(Color32::from_rgb(240, 90, 80)));
        }
        None => {}
    }
}
