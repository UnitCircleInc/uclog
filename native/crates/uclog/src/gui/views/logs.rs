//! Log view: filters, a virtualised table, per-source colours, selection, copy and CSV export
//! (GUI plan §3, plans README §5.6).

use egui::{Color32, RichText, Ui};
use egui_extras::{Column, TableBuilder};

use crate::front::logs::{Row, LEVELS};
use crate::gui::app::GuiApp;

pub fn level_color(level: u8) -> Color32 {
    match level {
        0 => Color32::from_rgb(140, 140, 150),
        1 => Color32::from_rgb(110, 180, 230),
        2 => Color32::from_rgb(230, 190, 70),
        _ => Color32::from_rgb(240, 90, 80),
    }
}

/// The colour key of a source: its file name without directory and extension (as the legacy GUI).
pub fn source_key(file: &str) -> String {
    let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
    base.split('.').next().unwrap_or(base).to_string()
}

/// A source's background: the saved colour, or one derived from its name.
pub fn source_color(app: &GuiApp, file: &str) -> Color32 {
    let k = source_key(file);
    let [r, g, b] = app.settings.gui.log_colors.get(&k).copied().unwrap_or_else(|| derived(&k));
    Color32::from_rgba_unmultiplied(r, g, b, 36)
}

fn derived(k: &str) -> [u8; 3] {
    // FNV-1a → hue; fixed saturation and value.
    let h = k.bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193));
    let c = egui::ecolor::Hsva::new((h % 360) as f32 / 360.0, 0.55, 0.75, 1.0);
    let c: Color32 = c.into();
    [c.r(), c.g(), c.b()]
}

fn row_text(r: &Row) -> String {
    r.plain()
}

pub fn ui(app: &mut GuiApp, ui: &mut Ui) {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("log-level")
            .selected_text(format!("≥ {}", LEVELS[app.log_view.level as usize]))
            .width(90.0)
            .show_ui(ui, |ui| {
                for (i, l) in LEVELS.iter().enumerate() {
                    changed |= ui.selectable_value(&mut app.log_view.level, i as u8, *l).changed();
                }
            });
        ui.label("Source");
        changed |= ui
            .add(egui::TextEdit::singleline(&mut app.log_view.source).desired_width(110.0).hint_text("file"))
            .changed();
        ui.label("Line");
        changed |= ui.add(egui::TextEdit::singleline(&mut app.log_view.line).desired_width(40.0)).changed();
        ui.label("Search");
        let mut search = egui::TextEdit::singleline(&mut app.log_view.search).desired_width(160.0).hint_text("regex");
        if app.log_view.search_error.is_some() {
            search = search.text_color(Color32::from_rgb(240, 90, 80));
        }
        let r = ui.add(search);
        changed |= r.changed();
        if let Some(e) = &app.log_view.search_error {
            r.on_hover_text(e);
        }
        if ui.button("✖").on_hover_text("Clear filters").clicked() {
            app.log_view.level = 0;
            app.log_view.source.clear();
            app.log_view.line.clear();
            app.log_view.search.clear();
            changed = true;
        }
        ui.separator();
        ui.toggle_value(&mut app.log_view.follow, "Follow").on_hover_text("Scroll to new lines");
        ui.checkbox(&mut app.logs.discard_filtered, "Discard filtered")
            .on_hover_text("Drop lines that do not pass the filter instead of keeping them");
        if ui.button("Clear").clicked() {
            app.logs.clear();
            app.log_view.selected.clear();
        }
        if ui.button("Copy").on_hover_text("Selected rows, or all shown").clicked() {
            let t = selected_text(app);
            ui.ctx().copy_text(t);
            app.say("copied");
        }
        if ui.button("Export CSV").clicked() {
            let csv = app.logs.csv();
            app.export("logs.csv", &csv);
        }
        ui.label(RichText::new(format!("{} of {}", app.logs.visible_len(), app.logs.len())).weak());
    });
    if changed {
        app.set_filter();
    }
    if !app.logs.error_counts.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("errors:").color(level_color(3)));
            let counts: Vec<String> = app.logs.error_counts.iter().map(|(k, v)| format!("{k} {v}")).collect();
            ui.label(RichText::new(counts.join(" · ")).color(level_color(3)));
        });
    }
    ui.separator();
    table(app, ui);
}

fn selected_text(app: &GuiApp) -> String {
    let mut out = vec![];
    for i in 0..app.logs.visible_len() {
        let r = app.logs.visible(i).unwrap();
        if app.log_view.selected.is_empty() || app.log_view.selected.contains(&r.seq) {
            out.push(row_text(r));
        }
    }
    out.join("\n")
}

fn table(app: &mut GuiApp, ui: &mut Ui) {
    let n = app.logs.visible_len();
    let h = ui.text_style_height(&egui::TextStyle::Monospace) + 4.0;
    let mut clicked: Option<(usize, egui::Modifiers)> = None;
    let mut filter_source: Option<String> = None;
    let mut filter_line: Option<(String, String)> = None;
    let mut set_color: Option<(String, Option<[u8; 3]>)> = None;
    let mut copy: Option<String> = None;
    let mods = ui.input(|i| i.modifiers);
    let mut t = TableBuilder::new(ui)
        .striped(false)
        .resizable(true)
        .sense(egui::Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(72.0))
        .column(Column::exact(52.0))
        .column(Column::initial(170.0).at_least(60.0).clip(true))
        .column(Column::remainder().clip(true))
        .min_scrolled_height(0.0)
        .stick_to_bottom(app.log_view.follow);
    if app.log_view.follow && n > 0 {
        t = t.scroll_to_row(n - 1, Some(egui::Align::BOTTOM));
    }
    t.header(h, |mut hr| {
        for c in ["Time", "Level", "Source", "Text"] {
            hr.col(|ui| {
                ui.strong(c);
            });
        }
    })
    .body(|body| {
        body.rows(h, n, |mut row| {
            let i = row.index();
            let Some(r) = app.logs.visible(i) else { return };
            let selected = app.log_view.selected.contains(&r.seq);
            row.set_selected(selected);
            let bg = source_color(app, &r.file);
            let mono = |s: String| RichText::new(s).monospace();
            let color = if r.error.is_some() { level_color(3) } else { level_color(r.level) };
            let paint = |ui: &mut Ui| {
                if !selected {
                    ui.painter().rect_filled(ui.max_rect().expand2(egui::vec2(4.0, 2.0)), 0.0, bg);
                }
            };
            row.col(|ui| {
                paint(ui);
                ui.label(mono(format!("{:9.3}", r.ts)));
            });
            row.col(|ui| {
                paint(ui);
                ui.label(mono(LEVELS[r.level as usize].to_string()).color(color));
            });
            row.col(|ui| {
                paint(ui);
                ui.label(mono(r.source()));
            });
            row.col(|ui| {
                paint(ui);
                let mut t = mono(r.text.clone());
                if r.error.is_some() {
                    t = t.color(color);
                }
                ui.add(egui::Label::new(t).truncate());
            });
            let resp = row.response();
            if resp.clicked() {
                clicked = Some((i, mods));
            }
            if resp.double_clicked() {
                filter_source = Some(r.file.clone());
            }
            let (file, line, key) = (r.file.clone(), r.line.clone(), source_key(&r.file));
            let text = row_text(r);
            resp.context_menu(|ui| {
                if ui.button(format!("Show only {file}")).clicked() {
                    filter_source = Some(file.clone());
                    ui.close();
                }
                if !line.is_empty() && ui.button(format!("Show only {file}:{line}")).clicked() {
                    filter_line = Some((file.clone(), line.clone()));
                    ui.close();
                }
                if ui.button("Copy row").clicked() {
                    copy = Some(text.clone());
                    ui.close();
                }
                ui.separator();
                ui.label(format!("Colour for '{key}'"));
                let mut c = {
                    let c = source_color(app, &file);
                    [c.r(), c.g(), c.b()]
                };
                // The colour is stored without alpha; show it opaque.
                let mut rgb = Color32::from_rgb(c[0], c[1], c[2]);
                if egui::color_picker::color_edit_button_srgba(ui, &mut rgb, egui::color_picker::Alpha::Opaque)
                    .changed()
                {
                    c = [rgb.r(), rgb.g(), rgb.b()];
                    set_color = Some((key.clone(), Some(c)));
                }
                if app.settings.gui.log_colors.contains_key(&key) && ui.button("Default colour").clicked() {
                    set_color = Some((key.clone(), None));
                    ui.close();
                }
            });
        });
    });
    if let Some((i, m)) = clicked {
        let seq = app.logs.visible(i).map(|r| r.seq);
        if let Some(seq) = seq {
            if m.shift {
                let a = app.log_view.anchor.unwrap_or(i);
                let (lo, hi) = (a.min(i), a.max(i));
                if !m.command {
                    app.log_view.selected.clear();
                }
                for k in lo..=hi {
                    if let Some(r) = app.logs.visible(k) {
                        app.log_view.selected.insert(r.seq);
                    }
                }
            } else if m.command {
                if !app.log_view.selected.remove(&seq) {
                    app.log_view.selected.insert(seq);
                }
                app.log_view.anchor = Some(i);
            } else {
                let only = app.log_view.selected.len() == 1 && app.log_view.selected.contains(&seq);
                app.log_view.selected.clear();
                if !only {
                    app.log_view.selected.insert(seq);
                }
                app.log_view.anchor = Some(i);
            }
            // Selecting stops following, so the row stays in view.
            app.log_view.follow = false;
        }
    }
    if let Some(f) = filter_source {
        app.log_view.source = f;
        app.log_view.line.clear();
        app.set_filter();
    }
    if let Some((f, l)) = filter_line {
        app.log_view.source = f;
        app.log_view.line = l;
        app.set_filter();
    }
    if let Some((k, c)) = set_color {
        match c {
            Some(c) => app.settings.gui.log_colors.insert(k, c),
            None => app.settings.gui.log_colors.remove(&k),
        };
        app.save_settings();
    }
    if let Some(t) = copy {
        ui.ctx().copy_text(t);
        app.say("copied");
    }
}
