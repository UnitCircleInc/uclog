//! Connect dialog (GUI plan §3): local Serial/HID devices, local and mDNS hubs (busy shown), a BLE
//! scan, and a typed address. Clicking a row fills in the address; double-clicking connects.

use egui::{Color32, RichText};
use egui_extras::{Column, TableBuilder};
use uclog_link::{Device, LinkSpec};

use super::app::GuiApp;
use super::backend::Cmd;

pub fn dialog(app: &mut GuiApp, ctx: &egui::Context) {
    if !app.connect.open {
        return;
    }
    let mut open = true;
    let mut connect: Option<String> = None;
    egui::Window::new("Connect").open(&mut open).default_width(760.0).default_height(360.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.add(egui::TextEdit::singleline(&mut app.connect.filter).desired_width(160.0));
            if ui.button("⟳ Refresh").clicked() {
                app.backend.send(Cmd::RefreshDevices);
            }
            let scan = if app.connect.ble_scanning { "Scanning BLE (5 s)…" } else { "Scan BLE" };
            if ui.add_enabled(!app.connect.ble_scanning, egui::Button::new(scan)).clicked() {
                app.connect.ble_scanning = true;
                app.backend.send(Cmd::BleScan);
            }
            if let Some(e) = &app.connect.ble_error {
                ui.label(RichText::new(format!("BLE: {e}")).color(Color32::from_rgb(240, 90, 80)));
            }
        });
        let f = app.connect.filter.to_lowercase();
        let rows: Vec<Device> = app
            .connect
            .devices
            .iter()
            .chain(app.connect.ble.iter())
            .filter(|d| {
                f.is_empty()
                    || [&d.uri, &d.name, &d.serial, &d.description].iter().any(|s| s.to_lowercase().contains(&f))
            })
            .cloned()
            .collect();
        ui.separator();
        let h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        TableBuilder::new(ui)
            .striped(true)
            .sense(egui::Sense::click())
            .column(Column::exact(60.0))
            .column(Column::initial(220.0).clip(true))
            .column(Column::initial(160.0).clip(true))
            .column(Column::initial(150.0).clip(true))
            .column(Column::remainder().clip(true))
            .max_scroll_height(220.0)
            .header(h, |mut hr| {
                for c in ["Kind", "Address", "Name", "Serial", "Description"] {
                    hr.col(|ui| {
                        ui.strong(c);
                    });
                }
            })
            .body(|body| {
                body.rows(h, rows.len(), |mut row| {
                    let d = &rows[row.index()];
                    row.set_selected(app.connect.selected.as_deref() == Some(d.uri.as_str()));
                    // Selectable text would take the clicks meant for the row.
                    let cell = |ui: &mut egui::Ui, t: RichText| {
                        ui.add(egui::Label::new(t).selectable(false));
                    };
                    row.col(|ui| cell(ui, d.kind.as_str().into()));
                    row.col(|ui| cell(ui, d.uri.as_str().into()));
                    row.col(|ui| cell(ui, d.name.as_str().into()));
                    row.col(|ui| cell(ui, d.serial.as_str().into()));
                    row.col(|ui| {
                        if d.busy == Some(true) {
                            cell(ui, RichText::new("busy").color(Color32::from_rgb(230, 150, 60)));
                        }
                        cell(ui, d.description.as_str().into());
                    });
                    let r = row.response();
                    if r.clicked() {
                        app.connect.selected = Some(d.uri.clone());
                        app.connect.address = d.uri.clone();
                    }
                    if r.double_clicked() {
                        connect = Some(d.uri.clone());
                    }
                });
            });
        if rows.is_empty() {
            ui.label(RichText::new("No devices found. Check the cable, or type an address below.").weak());
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Address");
            let r = ui.add(
                egui::TextEdit::singleline(&mut app.connect.address)
                    .desired_width(300.0)
                    .hint_text("tcp://host:9000, udp://ip:port, serial:/dev/…"),
            );
            if r.changed() {
                app.connect.selected = None;
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ok = !app.connect.address.trim().is_empty();
            if (ui.add_enabled(ok, egui::Button::new("Connect")).clicked() || enter) && ok {
                connect = Some(app.connect.address.trim().to_string());
            }
            if ui.add_enabled(app.conn.uri.is_some(), egui::Button::new("Disconnect")).clicked() {
                app.backend.send(Cmd::Disconnect);
            }
        });
    });
    if let Some(u) = connect {
        match u.parse::<LinkSpec>() {
            Ok(spec) => {
                app.backend.send(Cmd::Connect(spec));
                app.connect.open = false;
                return;
            }
            Err(e) => app.say(format!("{u}: {e}")),
        }
    }
    app.connect.open &= open;
}
