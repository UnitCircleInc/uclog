//! Drawing (ratatui). Reads [`App`]; never changes it.

use std::time::Duration;

use ansi_to_tui::IntoText;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use ratatui::Frame;
use uclog_proto::cbor::{self, Value};
use uclog_session::{FwProgress, Liveness};

use super::app::{button_args, App, Modal, PickFor, Tab, SETTINGS_FIELDS, TABS};
use super::lineedit::LineEdit;
use super::logs::{Row, LEVELS};

const SOURCE_COLOURS: [Color; 8] = [
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::LightCyan,
    Color::LightGreen,
    Color::LightMagenta,
];

fn source_colour(file: &str) -> Color {
    let h = file.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    SOURCE_COLOURS[(h % SOURCE_COLOURS.len() as u32) as usize]
}

fn level_style(level: u8, error: bool) -> Style {
    if error {
        return Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
    }
    match level {
        0 => Style::new().fg(Color::DarkGray),
        1 => Style::new(),
        2 => Style::new().fg(Color::Yellow),
        3 => Style::new().fg(Color::Red),
        _ => Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let [top, tabs, body, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)])
            .areas(f.area());
    draw_status(f, app, top);
    let titles: Vec<Line> = TABS.iter().map(|(_, t)| Line::from(*t)).collect();
    let sel = TABS.iter().position(|(t, _)| *t == app.tab).unwrap_or(0);
    f.render_widget(Tabs::new(titles).select(sel).highlight_style(Style::new().reversed()).divider("│"), tabs);
    match app.tab {
        Tab::Logs => {
            let [l, t] = Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)]).areas(body);
            draw_logs(f, app, l);
            draw_terminal(f, app, t, app.logs_term_focus);
        }
        Tab::Terminal => draw_terminal(f, app, body, true),
        Tab::Commands => draw_commands(f, app, body),
        Tab::Device => draw_device(f, app, body),
        Tab::Fw => draw_fw(f, app, body),
    }
    draw_bottom(f, app, bottom);
    match &app.modal {
        Modal::None => {}
        Modal::Help => draw_help(f),
        Modal::Connect => draw_connect(f, app),
        Modal::Settings => draw_settings(f, app),
        Modal::Picker(p, why) => draw_picker(f, p, *why),
        Modal::Prompt(p) => draw_prompt(f, &p.title, &p.input, p.secret),
    }
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let c = &app.conn;
    let (dot, state) = if let Some(u) = &c.connecting {
        (Span::raw("◌").yellow(), format!("connecting to {u}…"))
    } else {
        match c.liveness {
            Some(Liveness::Alive) => (Span::raw("●").green(), "live".to_string()),
            Some(Liveness::NotResponding) => (Span::raw("●").red(), "device not responding".into()),
            Some(Liveness::Waiting) => (
                Span::raw("◌").yellow(),
                if c.uri.as_deref().is_some_and(|u| u.starts_with("tcp:") || u.starts_with("ws")) {
                    "waiting for data (hub approval?)".into()
                } else {
                    "waiting for data".into()
                },
            ),
            None => (Span::raw("○").dark_gray(), c.closed.clone().unwrap_or_else(|| "not connected (F2)".into())),
        }
    };
    let logdata = match &app.device.logdata {
        Some(s) => format!("logdata: {} ✓", s.split_whitespace().next().unwrap_or("")),
        None if app.device.info.is_some() => "logdata: none".into(),
        None => "logdata: –".into(),
    };
    let mut spans = vec![
        Span::raw(" uclog ").bold(),
        Span::raw("─ "),
        Span::raw(if c.description.is_empty() { "–".into() } else { c.description.clone() }),
        Span::raw(" ─ "),
        dot,
        Span::raw(format!(" {state} ─ {logdata} ─ ")),
    ];
    let errs = app.logs.errors();
    spans.push(if errs > 0 { Span::raw(format!("errors: {errs}")).red() } else { Span::raw("errors: 0") });
    if app.plot_frames > 0 {
        spans.push(Span::raw(format!(" ─ plot data: {} frames (use uclog gui or the web version)", app.plot_frames)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(Style::new().bg(Color::DarkGray).fg(Color::White)), area);
}

fn draw_bottom(f: &mut Frame, app: &App, area: Rect) {
    let msg = app.status.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(6)).map(|(s, _)| s.clone());
    let hint = match app.tab {
        Tab::Logs if !app.logs_term_focus => {
            "+/- level  f source  n line  / search  s source of row  x clear  p pause  d discard  c copy  Tab terminal"
        }
        Tab::Logs | Tab::Terminal => {
            "Enter send  ↑↓ history  PgUp/PgDn scroll  ^T char mode  F6–F12 buttons  Esc back  Alt+1–5 views"
        }
        Tab::Commands => "↑↓ select  Enter press  Tab type a text command",
        Tab::Device => "l load logdata  c copy app_hash",
        Tab::Fw => "Enter choose image  Esc cancel",
    };
    let line = match msg {
        Some(m) => Line::from(vec![Span::raw(format!(" {m} ")).reversed()]),
        None => Line::from(vec![
            Span::raw(" F1 help  F2 connect  F3 logdata  ^E export  ^L clear  ^S settings  ^Q quit │ ").dark_gray(),
            Span::raw(hint).dark_gray(),
        ]),
    };
    f.render_widget(Paragraph::new(line), area);
}

fn log_line(r: &Row, width: usize, selected: bool) -> Line<'static> {
    let src = r.source();
    let src_w = 24usize;
    let n = src.chars().count();
    let src = if n > src_w { format!("…{}", src.chars().skip(n - (src_w - 1)).collect::<String>()) } else { src };
    let style = level_style(r.level, r.error.is_some());
    let text_w = width.saturating_sub(10 + 6 + src_w + 2);
    let text: String = r.text.chars().take(text_w.max(10)).collect();
    let mut l = Line::from(vec![
        Span::raw(format!("{:>9.3} ", r.ts)).dark_gray(),
        Span::styled(format!("{:<5} ", LEVELS[r.level as usize]), style),
        Span::styled(
            format!("{src:<src_w$} "),
            Style::new().fg(if r.error.is_some() { Color::Red } else { source_colour(&r.file) }),
        ),
        Span::styled(text, style),
    ]);
    if selected {
        l = l.style(Style::new().reversed());
    }
    l
}

fn draw_logs(f: &mut Frame, app: &App, area: Rect) {
    let n = app.logs.visible_len();
    let mut title = format!(" Logs {}/{} ", n, app.logs.len());
    if app.logs.filter.is_active() {
        title.push_str(&format!("─ filter: {} ", app.logs.filter.describe()));
    }
    if app.paused {
        title.push_str("─ PAUSED ");
    }
    if app.logs.discard_filtered {
        title.push_str(&format!("─ discarding ({}) ", app.logs.discarded));
    }
    let block = Block::default().borders(Borders::ALL).title(title).border_style(if app.logs_term_focus {
        Style::new().dark_gray()
    } else {
        Style::new()
    });
    let inner = block.inner(area);
    f.render_widget(block, area);
    let h = inner.height as usize;
    let last = app.log_sel.unwrap_or(n.saturating_sub(1));
    // Selected row stays visible; follow mode shows the newest rows.
    let start = if app.log_sel.is_none() { n.saturating_sub(h) } else { (last + 1).saturating_sub(h) };
    let lines: Vec<Line> = (start..(start + h).min(n))
        .filter_map(|i| app.logs.visible(i).map(|r| log_line(r, inner.width as usize, app.log_sel == Some(i))))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_terminal(f: &mut Frame, app: &App, area: Rect, focused: bool) {
    let t = &app.term;
    let mode = if t.stdin_wanted {
        "stdin (semihost)"
    } else if t.raw {
        "character mode"
    } else {
        "line mode"
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Terminal (port 61) ─ {mode} "))
        .border_style(if focused { Style::new() } else { Style::new().dark_gray() });
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [out, input] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let h = out.height as usize;
    let total = t.lines.len() + 1;
    let end = total.saturating_sub(t.scroll);
    let start = end.saturating_sub(h);
    let mut src = String::new();
    for i in start..end {
        if i < t.lines.len() {
            src.push_str(&t.lines[i]);
            src.push('\n');
        } else {
            src.push_str(&t.partial);
        }
    }
    let text = src.as_bytes().into_text().unwrap_or_else(|_| Text::raw(src.clone()));
    f.render_widget(Paragraph::new(text), out);
    // Buttons hint and the input line.
    let prompt = if t.stdin_wanted {
        "stdin> "
    } else if t.raw {
        "keys → device "
    } else {
        "> "
    };
    let buttons: String = app
        .settings
        .terminal
        .buttons
        .iter()
        .take(7)
        .enumerate()
        .map(|(i, b)| format!(" F{} {}", i + 6, b.label))
        .collect();
    let mut spans = vec![Span::raw(prompt).bold()];
    if !t.raw || t.stdin_wanted {
        spans.push(Span::raw(t.input.text.clone()));
    }
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = (input.width as usize).saturating_sub(used + buttons.chars().count());
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::raw(buttons).dark_gray());
    f.render_widget(Paragraph::new(Line::from(spans)), input);
    if focused && matches!(app.modal, Modal::None) && (!t.raw || t.stdin_wanted) {
        let x = input.x + prompt.chars().count() as u16 + t.input.cursor as u16;
        f.set_cursor_position((x.min(input.right().saturating_sub(1)), input.y));
    }
}

fn draw_commands(f: &mut Frame, app: &App, area: Rect) {
    let [left, right] = Layout::horizontal([Constraint::Length(32), Constraint::Min(20)]).areas(area);
    let items: Vec<ListItem> = app
        .settings
        .commands
        .buttons
        .iter()
        .map(|b| {
            let args = button_args(b);
            let a = if args.is_empty() {
                String::new()
            } else {
                format!(" ({})", args.iter().map(|x| x.1.as_str()).collect::<Vec<_>>().join(" "))
            };
            ListItem::new(format!("{}{a}", b.label))
        })
        .collect();
    let empty = items.is_empty();
    let mut st = ListState::default().with_selected((!empty).then_some(app.cmds.selected));
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Buttons (port 0) "))
        .highlight_style(Style::new().reversed());
    f.render_stateful_widget(list, left, &mut st);
    if empty {
        let inner = Block::default().borders(Borders::ALL).inner(left);
        f.render_widget(
            Paragraph::new("No buttons.\nAdd commands.buttons in the settings file (^S shows its path).")
                .wrap(Wrap { trim: true })
                .dark_gray(),
            inner,
        );
    }
    let block = Block::default().borders(Borders::ALL).title(" Responses ");
    let inner = block.inner(right);
    f.render_widget(block, right);
    let [out, input] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let h = out.height as usize;
    let lines: Vec<Line> = app
        .cmds
        .responses
        .iter()
        .skip(app.cmds.responses.len().saturating_sub(h))
        .map(|l| Line::raw(l.clone()))
        .collect();
    f.render_widget(Paragraph::new(lines), out);
    let style = if app.cmds.typing { Style::new() } else { Style::new().dark_gray() };
    f.render_widget(Paragraph::new(format!("text> {}", app.cmds.input.text)).style(style), input);
    if app.cmds.typing && matches!(app.modal, Modal::None) {
        f.set_cursor_position((input.x + 6 + app.cmds.input.cursor as u16, input.y));
    }
}

fn value_lines(v: &Value) -> Vec<Line<'static>> {
    match v {
        Value::Map(m) => m
            .iter()
            .map(|(k, v)| {
                let key = k.as_text().map(str::to_string).unwrap_or_else(|| cbor::display(k));
                let val = match v {
                    Value::Bytes(b) => hex::encode(b),
                    other => other.as_text().map(str::to_string).unwrap_or_else(|| cbor::display(other)),
                };
                Line::from(vec![Span::raw(format!("  {key:<14} ")).bold(), Span::raw(val)])
            })
            .collect(),
        other => vec![Line::raw(format!("  {}", cbor::display(other)))],
    }
}

fn draw_device(f: &mut Frame, app: &App, area: Rect) {
    let mut lines = vec![Line::raw("Connection").bold()];
    let c = &app.conn;
    lines.push(Line::raw(format!("  {:<14} {}", "device", c.uri.clone().unwrap_or_else(|| "–".into()))));
    lines.push(Line::raw(format!("  {:<14} {}", "link", if c.description.is_empty() { "–" } else { &c.description })));
    lines.push(Line::raw(""));
    lines.push(Line::raw("Device info (port 62)").bold());
    match &app.device.info {
        Some(v) => lines.extend(value_lines(v)),
        None => lines.push(Line::raw("  not received yet").dark_gray()),
    }
    lines.push(Line::raw(""));
    lines.push(Line::raw("Logdata").bold());
    lines.push(Line::raw(format!(
        "  {:<14} {}",
        "source",
        app.device.logdata.clone().unwrap_or_else(|| "none (press l to load a .logdata or .elf)".into())
    )));
    if !app.device.targets.is_empty() {
        lines.push(Line::raw(format!("  {:<14} {:?}", "targets", app.device.targets)));
    }
    lines.push(Line::raw(""));
    lines.push(Line::raw("Counters").bold());
    lines.push(Line::raw(format!("  {:<14} {} received, {} stored", "logs", app.logs.received, app.logs.len())));
    for (k, n) in &app.logs.error_counts {
        lines.push(Line::raw(format!("  {k:<14} {n}")).red());
    }
    for (p, n) in &app.port_counts {
        lines.push(Line::raw(format!("  {:<14} {n} frames", format!("port {p}"))));
    }
    if !app.device.notes.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::raw("Notes").bold());
        for n in app.device.notes.iter().rev().take(10) {
            lines.push(Line::raw(format!("  {n}")).dark_gray());
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Device "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub fn fw_ratio(p: &FwProgress) -> (f64, String) {
    match p {
        FwProgress::Probing => (0.0, "probing port 59 / 3…".into()),
        FwProgress::Port(n) => (0.0, format!("using port {n}")),
        FwProgress::Starting => (0.0, "erasing (fw-start)…".into()),
        FwProgress::Block { done, total } => (*done as f64 / (*total).max(1) as f64, format!("block {done}/{total}")),
        FwProgress::Scheduling => (1.0, "scheduling…".into()),
        FwProgress::Done => (1.0, "done".into()),
    }
}

fn draw_fw(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" FW Update (port 59, falling back to port 3) ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [info, gauge, result] =
        Layout::vertical([Constraint::Length(3), Constraint::Length(3), Constraint::Min(1)]).areas(inner);
    let file = app
        .fw
        .file
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "none — press Enter to choose a signed .bin".into());
    f.render_widget(
        Paragraph::new(vec![
            Line::raw(format!("Image: {file}")),
            Line::raw(if app.fw.running { "Esc cancels." } else { "" }).dark_gray(),
        ]),
        info,
    );
    if let Some(p) = &app.fw.progress {
        let (r, label) = fw_ratio(p);
        f.render_widget(
            Gauge::default().block(Block::default().borders(Borders::ALL)).ratio(r.clamp(0.0, 1.0)).label(label),
            gauge,
        );
    }
    if let Some(r) = &app.fw.result {
        let l = match r {
            Ok(s) => Line::raw(s.clone()).green(),
            Err(e) => Line::raw(format!("Failed: {e}")).red(),
        };
        f.render_widget(Paragraph::new(l).wrap(Wrap { trim: true }), result);
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn draw_help(f: &mut Frame) {
    let area = centered(f.area(), 78, 30);
    f.render_widget(Clear, area);
    let rows = [
        ("F1", "this help"),
        ("F2", "connect: devices and hubs (s scan BLE, a add address, r refresh, d disconnect)"),
        ("F3", "load logdata (.logdata or ELF)"),
        ("F5 / 5", "FW update"),
        ("1–5", "views (when not typing; Alt+1–5 always)"),
        ("Ctrl+E", "export the current view (logs as CSV)"),
        ("Ctrl+L", "clear the current view"),
        ("Ctrl+S", "settings"),
        ("Ctrl+T", "terminal: character mode / line mode"),
        ("F6–F12", "terminal buttons (settings: terminal.buttons)"),
        ("Ctrl+Q", "quit (q also quits when not typing)"),
        ("", ""),
        ("Logs", "↑↓ PgUp PgDn Home select · End follow · +/- level · f source · n line"),
        ("", "/ regex search · s filter on the row's source · x clear filters"),
        ("", "p pause · d discard filtered lines · c copy row · Tab terminal below"),
        ("Terminal", "Enter sends a line · ↑↓ history · PgUp/PgDn scroll · Esc back to Logs"),
        ("Commands", "↑↓ Enter press a button ($args are asked) · Tab type a text command"),
        ("Device", "l load logdata · c copy app_hash"),
    ];
    let lines: Vec<Line> =
        rows.iter().map(|(k, v)| Line::from(vec![Span::raw(format!("{k:<10} ")).bold(), Span::raw(*v)])).collect();
    f.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Help (Esc) ")), area);
}

fn draw_connect(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 110, 20);
    f.render_widget(Clear, area);
    let items: Vec<ListItem> = app
        .devices
        .iter()
        .map(|d| {
            let busy = if d.busy == Some(true) { "[busy] " } else { "" };
            ListItem::new(format!(
                "{:<6} {:<36} {busy}{:<24} {:<18} {}",
                d.kind.as_str(),
                d.uri,
                d.name,
                d.serial,
                d.description
            ))
        })
        .collect();
    let title = format!(
        " Connect ─ Enter connect · s scan BLE{} · a add address · r refresh · d disconnect · Esc ",
        if app.scanning { " (scanning…)" } else { "" }
    );
    let empty = items.is_empty();
    let mut st = ListState::default().with_selected((!empty).then_some(app.devices_sel));
    f.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::new().reversed()),
        area,
        &mut st,
    );
    if empty {
        let inner = Block::default().borders(Borders::ALL).inner(area);
        f.render_widget(
            Paragraph::new("No devices or hubs found. Press a to type an address, s to scan BLE.").dark_gray(),
            inner,
        );
    }
}

fn draw_settings(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 100, (SETTINGS_FIELDS.len() + 6) as u16);
    f.render_widget(Clear, area);
    let items: Vec<ListItem> = SETTINGS_FIELDS
        .iter()
        .enumerate()
        .map(|(i, name)| ListItem::new(format!("{name:<40} {}", app.setting_value(i))))
        .collect();
    let mut st = ListState::default().with_selected(Some(app.settings_sel));
    let block = Block::default().borders(Borders::ALL).title(" Settings ─ Enter edit · s save · Esc close ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [list, note] = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(inner);
    f.render_stateful_widget(List::new(items).highlight_style(Style::new().reversed()), list, &mut st);
    f.render_widget(
        Paragraph::new("Logdata settings apply to the next connection. Buttons: edit the settings file.").dark_gray(),
        note,
    );
}

fn draw_picker(f: &mut Frame, p: &super::picker::Picker, why: PickFor) {
    let area = centered(f.area(), 100, 24);
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title(format!(
        " {} ─ {} ─ Enter open · Tab complete · Esc cancel ",
        p.title,
        match why {
            PickFor::Logdata => "logdata",
            PickFor::FwImage => "FW image",
        }
    ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [dir, list, input] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    f.render_widget(Paragraph::new(p.error.clone().unwrap_or_else(|| p.dir.display().to_string())).dark_gray(), dir);
    let items: Vec<ListItem> = p.entries.iter().map(|e| ListItem::new(e.0.clone())).collect();
    let mut st = ListState::default().with_selected(Some(p.selected));
    f.render_stateful_widget(List::new(items).highlight_style(Style::new().reversed()), list, &mut st);
    f.render_widget(Paragraph::new(format!("path> {}", p.path.text)), input);
    f.set_cursor_position((input.x + 6 + p.path.cursor as u16, input.y));
}

fn draw_prompt(f: &mut Frame, title: &str, input: &LineEdit, secret: bool) {
    let area = centered(f.area(), 90, 3);
    f.render_widget(Clear, area);
    let text = if secret { "•".repeat(input.text.chars().count()) } else { input.text.clone() };
    let block = Block::default().borders(Borders::ALL).title(format!(" {title} "));
    let inner = block.inner(area);
    f.render_widget(Paragraph::new(text).block(block), area);
    f.set_cursor_position((inner.x + input.cursor as u16, inner.y));
}
