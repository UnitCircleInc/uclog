//! Device view (port 62): device info, the logdata in use, and counters.

use egui::{RichText, Ui};
use uclog_proto::cbor::{self, Value};

use crate::gui::app::GuiApp;

pub fn ui(app: &mut GuiApp, ui: &mut Ui) {
    egui::ScrollArea::vertical().id_salt("device").auto_shrink(false).show(ui, |ui| {
        egui::Grid::new("device-grid").num_columns(2).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
            ui.label("Connection");
            ui.label(app.conn.uri.clone().unwrap_or_else(|| "—".into()));
            ui.end_row();
            if !app.conn.description.is_empty() {
                ui.label("");
                ui.label(&app.conn.description);
                ui.end_row();
            }
            match &app.device.info {
                Some(Value::Map(m)) => {
                    for (k, v) in m {
                        let key = k.as_text().map(String::from).unwrap_or_else(|| cbor::display(k));
                        ui.label(key);
                        let text = match v {
                            Value::Bytes(b) => hex::encode(b),
                            Value::Text(t) => t.clone(),
                            v => cbor::display(v),
                        };
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(RichText::new(&text).monospace()).wrap());
                            if matches!(v, Value::Bytes(_)) && ui.small_button("Copy").clicked() {
                                ui.ctx().copy_text(text.clone());
                            }
                        });
                        ui.end_row();
                    }
                }
                _ => {
                    ui.label("Device info");
                    ui.label(RichText::new("none yet (it arrives on port 62)").weak());
                    ui.end_row();
                }
            }
            ui.label("Logdata");
            ui.label(app.device.logdata.clone().unwrap_or_else(|| "none".into()));
            ui.end_row();
            if !app.device.targets.is_empty() {
                ui.label("Targets");
                ui.label(app.device.targets.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", "));
                ui.end_row();
            }
        });
        if ui.button("Load logdata…").on_hover_text("A .logdata file or an ELF with .logdata_cbor").clicked() {
            app.load_logdata_dialog();
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Frames by port").strong());
        let counts: Vec<String> = app.port_counts.iter().map(|(p, n)| format!("{p}: {n}")).collect();
        ui.label(if counts.is_empty() { "—".into() } else { counts.join("   ") });
        ui.label(format!(
            "Logs received {}, shown {}, discarded {}",
            app.logs.received,
            app.logs.visible_len(),
            app.logs.discarded
        ));
        if !app.logs.error_counts.is_empty() {
            ui.label(RichText::new("Error records").strong());
            for (k, v) in &app.logs.error_counts {
                ui.label(format!("{k}: {v}"));
            }
        }
        if app.plots.frames > 0 {
            ui.label(format!(
                "Plot frames {} ({} for plots created before connecting, {} invalid)",
                app.plots.frames, app.plots.orphans, app.plots.errors
            ));
        }
        if !app.notes.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Notes").strong());
            for n in app.notes.iter().rev().take(30) {
                ui.label(RichText::new(n).weak());
            }
        }
    });
}
