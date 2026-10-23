//! Settings dialog (GUI plan §3): logdata service, USB filters, log lines, directories, on-connect
//! lines and the button lists (as JSON). The token goes to the OS keyring.

use egui::{Color32, RichText};

use super::app::{open_path, GuiApp};
use super::backend::Cmd;
use crate::settings::{self, Button};

pub fn dialog(app: &mut GuiApp, ctx: &egui::Context) {
    let Some(d) = &mut app.settings_dlg else { return };
    let mut open = d.open;
    let mut save = false;
    let mut cancel = false;
    egui::Window::new("Settings").open(&mut open).default_width(620.0).show(ctx, |ui| {
        egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
            egui::Grid::new("settings-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Logdata domain");
                let mut dom = d.s.logdata.domain.clone().unwrap_or_default();
                if ui.text_edit_singleline(&mut dom).changed() {
                    d.s.logdata.domain = (!dom.trim().is_empty()).then(|| dom.trim().to_string());
                }
                ui.end_row();
                ui.label("Logdata URL");
                let mut url = d.s.logdata.url.clone().unwrap_or_default();
                if ui.add(egui::TextEdit::singleline(&mut url).hint_text(crate::DEFAULT_LOGDATA_URL)).changed() {
                    d.s.logdata.url = (!url.trim().is_empty()).then(|| url.trim().to_string());
                }
                ui.end_row();
                ui.label("Logdata token");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.token).password(true).hint_text(if d.token_set {
                        "(saved in the keyring)"
                    } else {
                        ""
                    }));
                    if d.token_set && ui.button("Remove").clicked() {
                        match settings::set_keyring_token(None) {
                            Ok(()) => d.token_set = false,
                            Err(e) => d.error = Some(format!("keyring: {e}")),
                        }
                    }
                });
                ui.end_row();
                ui.label("USB filters");
                ui.add(egui::TextEdit::singleline(&mut d.usb).hint_text("VID:PID, … (empty: the defaults)"));
                ui.end_row();
                ui.label("Max log lines");
                ui.add(egui::DragValue::new(&mut d.s.max_log_lines).range(100..=10_000_000).speed(100));
                ui.end_row();
                dir_row(ui, "Files directory", &mut d.s.files_dir, "semihost files (sandbox)");
                dir_row(ui, "Export directory", &mut d.s.export_dir, "default: the working directory");
                ui.label("Terminal on connect");
                ui.add(
                    egui::TextEdit::multiline(&mut d.on_connect)
                        .desired_rows(2)
                        .hint_text("one line each, e.g. version s"),
                );
                ui.end_row();
            });
            ui.label(RichText::new("Terminal buttons (JSON: [{\"label\": …, \"cmd\": \"text\"}])").strong());
            ui.add(
                egui::TextEdit::multiline(&mut d.term_buttons)
                    .code_editor()
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            );
            ui.label(
                RichText::new("Command buttons (JSON; \"cmd\" is a text line or an object sent as CBOR, with $args)")
                    .strong(),
            );
            ui.add(
                egui::TextEdit::multiline(&mut d.cmd_buttons)
                    .code_editor()
                    .desired_rows(6)
                    .desired_width(f32::INFINITY),
            );
        });
        if let Some(e) = &d.error {
            ui.label(RichText::new(e).color(Color32::from_rgb(240, 90, 80)));
        }
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                save = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
            if let Some(p) = app.settings_path.clone() {
                if ui.button("Open settings file").on_hover_text(p.display().to_string()).clicked() {
                    open_path(&p);
                }
            }
        });
    });
    if save {
        let parse = |t: &str, what: &str| -> Result<Vec<Button>, String> {
            serde_json::from_str(t).map_err(|e| format!("{what}: {e}"))
        };
        let d = app.settings_dlg.as_mut().unwrap();
        match (parse(&d.term_buttons, "terminal buttons"), parse(&d.cmd_buttons, "command buttons")) {
            (Ok(tb), Ok(cb)) => {
                let mut s = d.s.clone();
                s.terminal.buttons = tb;
                s.commands.buttons = cb;
                s.usb_filters = d.usb.split([',', ' ']).filter(|x| !x.is_empty()).map(String::from).collect();
                s.terminal.on_connect =
                    d.on_connect.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
                // Keep what the GUI changed meanwhile.
                s.terminal.history = app.settings.terminal.history.clone();
                s.gui = app.settings.gui.clone();
                let token = std::mem::take(&mut d.token);
                if !token.trim().is_empty() {
                    if let Err(e) = settings::set_keyring_token(Some(token.trim())) {
                        d.error = Some(format!("keyring: {e}"));
                        return;
                    }
                }
                if s.max_log_lines != app.settings.max_log_lines {
                    app.logs.set_max(s.max_log_lines);
                }
                app.settings = s;
                app.save_settings();
                app.backend.send(Cmd::Settings(Box::new(app.settings.clone())));
                app.settings_dlg = None;
                app.say("settings saved");
            }
            (Err(e), _) | (_, Err(e)) => d.error = Some(e),
        }
        return;
    }
    if cancel || !open {
        app.settings_dlg = None;
    }
}

fn dir_row(ui: &mut egui::Ui, label: &str, v: &mut Option<std::path::PathBuf>, hint: &str) {
    ui.label(label);
    ui.horizontal(|ui| {
        let mut t = v.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        if ui.add(egui::TextEdit::singleline(&mut t).hint_text(hint).desired_width(300.0)).changed() {
            *v = (!t.trim().is_empty()).then(|| t.trim().into());
        }
        if ui.button("Browse…").clicked() {
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                *v = Some(d);
            }
        }
    });
    ui.end_row();
}
