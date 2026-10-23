//! Terminal view (port 61): ANSI colours, scrollback, an input line with history, and the
//! configured buttons. Semihost stdout/stderr appears here too, and stdin reads take the next line.

use egui::text::LayoutJob;
use egui::{Color32, FontId, RichText, TextFormat, Ui};
use uclog_proto::ports;

use crate::gui::app::GuiApp;
use crate::gui::backend::Cmd;
use crate::gui::term::{palette, Cell, Color, Style};

fn rgb(c: Color, default: Color32) -> Color32 {
    match c {
        Color::Default => default,
        Color::Idx(i) => {
            let (r, g, b) = palette(i);
            Color32::from_rgb(r, g, b)
        }
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

fn layout(line: &[Cell], font: &FontId, fg: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut run = String::new();
    let mut style: Option<Style> = None;
    let flush = |job: &mut LayoutJob, run: &mut String, s: Style| {
        if run.is_empty() {
            return;
        }
        let (mut f, mut b) = (rgb(s.fg, fg), rgb(s.bg, Color32::TRANSPARENT));
        if s.inverse {
            std::mem::swap(&mut f, &mut b);
            if f == Color32::TRANSPARENT {
                f = Color32::BLACK;
            }
            if b == Color32::TRANSPARENT {
                b = fg;
            }
        }
        if s.bold && s.fg == Color::Default {
            f = f.gamma_multiply(1.3);
        }
        job.append(run, 0.0, TextFormat { font_id: font.clone(), color: f, background: b, ..Default::default() });
        run.clear();
    };
    for c in line {
        if style != Some(c.style) {
            if let Some(s) = style {
                flush(&mut job, &mut run, s);
            }
            style = Some(c.style);
        }
        run.push(c.c);
    }
    if let Some(s) = style {
        flush(&mut job, &mut run, s);
    }
    job
}

pub fn ui(app: &mut GuiApp, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        for b in app.settings.terminal.buttons.clone() {
            if ui.button(&b.label).on_hover_text(b.cmd.to_string()).clicked() {
                app.send(ports::TERMINAL, crate::front::button_payload(&b, &[]));
            }
        }
        ui.separator();
        if ui.button("Clear").clicked() {
            app.term.clear();
        }
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(app.term.text());
            app.say("copied");
        }
        if ui.button("Export").clicked() {
            let t = app.term.text();
            app.export("terminal.txt", &t);
        }
    });
    ui.separator();
    let font = FontId::monospace(ui.text_style_height(&egui::TextStyle::Monospace) - 2.0);
    let row_h = ui.fonts_mut(|f| f.row_height(&font));
    let fg = ui.visuals().text_color();
    let input_h = ui.spacing().interact_size.y + 8.0;
    let lines = app.term.lines();
    egui::ScrollArea::both()
        .id_salt("term-scroll")
        .auto_shrink(false)
        .stick_to_bottom(true)
        .max_height(ui.available_height() - input_h)
        .show_rows(ui, row_h, lines.len(), |ui, range| {
            for i in range {
                let job = layout(&lines[i], &font, fg);
                ui.add(egui::Label::new(job).extend());
            }
        });
    input(app, ui);
}

fn input(app: &mut GuiApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let prompt = if app.term_view.stdin_wanted {
            RichText::new("stdin ›").color(Color32::from_rgb(230, 190, 70))
        } else {
            RichText::new("›")
        };
        ui.label(prompt);
        let id = egui::Id::new("term-input");
        let r = ui.add(
            egui::TextEdit::singleline(&mut app.term_view.input)
                .id(id)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .hint_text(if app.term_view.stdin_wanted {
                    "line for the device's stdin"
                } else {
                    "command, Enter to send"
                }),
        );
        if r.has_focus() {
            let (up, down) = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown)));
            let h = &app.settings.terminal.history;
            if up && !h.is_empty() {
                let i = app.term_view.hist.map_or(h.len() - 1, |i| i.saturating_sub(1));
                app.term_view.hist = Some(i);
                app.term_view.input = h[i].clone();
            }
            if down {
                match app.term_view.hist {
                    Some(i) if i + 1 < h.len() => {
                        app.term_view.hist = Some(i + 1);
                        app.term_view.input = h[i + 1].clone();
                    }
                    Some(_) => {
                        app.term_view.hist = None;
                        app.term_view.input.clear();
                    }
                    None => {}
                }
            }
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let line = std::mem::take(&mut app.term_view.input);
            app.term_view.hist = None;
            if app.term_view.stdin_wanted {
                app.term_view.stdin_wanted = false;
                app.term.feed(format!("{line}\r\n").as_bytes());
                app.backend.send(Cmd::Stdin(format!("{line}\n").into_bytes()));
            } else {
                app.send(ports::TERMINAL, format!("{line}\r").into_bytes());
                if !line.trim().is_empty() {
                    app.settings.push_history(&line);
                    app.save_settings();
                }
            }
            r.request_focus();
        }
    });
}
