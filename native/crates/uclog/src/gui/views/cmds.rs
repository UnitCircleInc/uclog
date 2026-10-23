//! Commands view (port 0): the configured buttons with their `$arg` fields, a free-form command
//! line (text, or a JSON object sent as CBOR), and the responses.

use egui::{RichText, Ui};
use uclog_proto::{cbor, ports};

use crate::front::{button_args, button_payload, json_to_cbor};
use crate::gui::app::GuiApp;

pub fn ui(app: &mut GuiApp, ui: &mut Ui) {
    let buttons = app.settings.commands.buttons.clone();
    if buttons.is_empty() {
        ui.label(RichText::new("No command buttons yet: add them in Settings (⌘,).").weak());
    }
    egui::Grid::new("cmd-buttons").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        for b in &buttons {
            let args = button_args(b);
            let clicked = ui.button(&b.label).on_hover_text(b.cmd.to_string()).clicked();
            ui.horizontal(|ui| {
                for (k, a) in &args {
                    ui.label(format!("{k} {a}"));
                    let v = app.cmds.args.entry((b.label.clone(), k.clone(), a.clone())).or_default();
                    ui.add(egui::TextEdit::singleline(v).desired_width(90.0));
                }
            });
            ui.end_row();
            if clicked {
                let answers: Vec<_> = args
                    .iter()
                    .map(|(k, a)| {
                        let v =
                            app.cmds.args.get(&(b.label.clone(), k.clone(), a.clone())).cloned().unwrap_or_default();
                        (k.clone(), a.clone(), v)
                    })
                    .collect();
                app.cmds.responses.push_back(format!("› {}", b.label));
                app.send(ports::COMMANDS, button_payload(b, &answers));
            }
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut app.cmds.free)
                .font(egui::TextStyle::Monospace)
                .desired_width(ui.available_width() - 60.0)
                .hint_text(r#"text line, or a JSON object such as {"cmd": "get-info"}"#),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Send").clicked() || enter) && !app.cmds.free.trim().is_empty() {
            let t = app.cmds.free.trim().to_string();
            let payload = match serde_json::from_str::<serde_json::Value>(&t) {
                Ok(v @ serde_json::Value::Object(_)) => cbor::to_vec(&json_to_cbor(&v)),
                _ => format!("{t}\r").into_bytes(),
            };
            app.cmds.responses.push_back(format!("› {t}"));
            app.send(ports::COMMANDS, payload);
            if enter {
                r.request_focus();
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Responses").strong());
        if ui.small_button("Clear").clicked() {
            app.cmds.responses.clear();
        }
        if ui.small_button("Copy").clicked() {
            ui.ctx().copy_text(app.cmds.responses.iter().cloned().collect::<Vec<_>>().join("\n"));
        }
    });
    egui::ScrollArea::vertical().id_salt("cmd-responses").auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
        for l in &app.cmds.responses {
            ui.label(RichText::new(l).monospace());
        }
    });
}
