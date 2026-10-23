//! Plot views (port 60): one dock tab per plot. Line plots with up to three y-axes (secondary
//! axes are mapped onto the primary axis' range and labelled on the right, GUI plan §8),
//! scatter/digital/infinite series, two draggable cursors, markers, auto-fit; heatmaps and
//! spectrograms as egui_plot heatmaps.

use egui::{Color32, RichText, Ui};
use egui_plot::{AxisHints, HLine, Heatmap, Legend, Line, LineStyle, Plot, PlotBounds, PlotPoint, Points, VLine};

use crate::gui::app::{safe_name, GuiApp};
use crate::gui::plots::{map, range, visible, GridPlot, Kind, LinePlot, Series};

pub struct PlotView {
    /// Fit the data on every frame, until the user pans or zooms.
    pub auto_fit: bool,
    pub cursors: Option<[f64; 2]>,
    /// Cursor under the pointer (last frame), and the one being dragged.
    near: Option<usize>,
    drag: Option<usize>,
    pub markers: Vec<f64>,
}

impl Default for PlotView {
    fn default() -> Self {
        PlotView { auto_fit: true, cursors: None, near: None, drag: None, markers: vec![] }
    }
}

const COLORS: [Color32; 9] = [
    Color32::from_rgb(86, 156, 214),
    Color32::from_rgb(230, 145, 56),
    Color32::from_rgb(106, 190, 120),
    Color32::from_rgb(220, 90, 90),
    Color32::from_rgb(170, 120, 210),
    Color32::from_rgb(150, 110, 80),
    Color32::from_rgb(220, 120, 190),
    Color32::from_rgb(140, 140, 140),
    Color32::from_rgb(190, 190, 60),
];

pub fn ui(app: &mut GuiApp, ui: &mut Ui, name: &str) {
    let GuiApp { plots, plot_views, .. } = app;
    let Some(p) = plots.get(name) else {
        ui.label(RichText::new("This plot does not exist (yet): the device creates it on port 60.").weak());
        return;
    };
    let view = plot_views.entry(name.to_string()).or_default();
    let mut export = false;
    ui.horizontal(|ui| {
        if ui
            .selectable_label(view.auto_fit, "Fit")
            .on_hover_text("Follow the data (panning or zooming stops it)")
            .clicked()
        {
            view.auto_fit = !view.auto_fit;
        }
        if matches!(p.kind, Kind::Line(_)) {
            if ui.selectable_label(view.cursors.is_some(), "Cursors").on_hover_text("Two cursors; drag them").clicked()
            {
                view.cursors = if view.cursors.is_some() { None } else { Some([f64::NAN; 2]) };
            }
            if !view.markers.is_empty() && ui.button("Clear markers").clicked() {
                view.markers.clear();
            }
            ui.label(RichText::new("⌘/Ctrl-click: marker · double-click: reset view").weak());
        }
        if ui.button("Export CSV").clicked() {
            export = true;
        }
    });
    match &p.kind {
        Kind::Line(l) => line_plot(ui, name, l, view),
        Kind::Grid(g) => grid_plot(ui, name, g, view),
    }
    if export {
        if let Some(csv) = app.plots.csv(name) {
            app.export(&format!("plot-{}.csv", safe_name(name)), &csv);
        }
    }
}

/// Each axis' value range: its limits, else its data (finite values), else 0–1.
fn axis_ranges(l: &LinePlot) -> Vec<(f64, f64)> {
    l.axes
        .iter()
        .map(|a| {
            let r = a.limits.or_else(|| range(a.series.iter().flat_map(|s| s.y.iter()))).unwrap_or((0.0, 1.0));
            if r.1 > r.0 {
                r
            } else {
                (r.0 - 0.5, r.1 + 0.5)
            }
        })
        .collect()
}

fn fmt(v: f64) -> String {
    let a = v.abs();
    if a != 0.0 && !(1e-3..1e6).contains(&a) {
        format!("{v:.3e}")
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// The series' y at the sample nearest to `x`.
fn value_at(s: &Series, x: f64) -> Option<f64> {
    if s.x.is_empty() {
        return s.y.back().copied();
    }
    let i = s.x.partition_point(|&v| v < x).min(s.x.len() - 1);
    let j = if i > 0 && (x - s.x[i - 1]).abs() < (s.x[i] - x).abs() { i - 1 } else { i };
    s.y.get(j).copied()
}

fn line_plot(ui: &mut Ui, name: &str, l: &LinePlot, view: &mut PlotView) {
    let ranges = axis_ranges(l);
    let primary = ranges.first().copied().unwrap_or((0.0, 1.0));
    let xr = range(l.axes.iter().flat_map(|a| a.series.iter()).flat_map(|s| s.x.iter())).unwrap_or((0.0, 1.0));
    let mut yaxes = vec![];
    for (k, a) in l.axes.iter().enumerate() {
        let mut h = AxisHints::new_y().label(a.label.clone());
        if k > 0 {
            let (ar, pr) = (ranges[k], primary);
            // Ticks sit at the primary axis' grid, so the mapped values are not round: show them
            // with the precision this axis' span calls for.
            let decimals = (2.0 - (ar.1 - ar.0).log10().floor()).max(0.0) as usize;
            h = h.placement(egui_plot::HPlacement::Right).formatter(move |m, _| {
                let s = format!("{:.*}", decimals, map(m.value, pr, ar));
                if s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') {
                    "0".into()
                } else {
                    s
                }
            });
        }
        yaxes.push(h);
    }
    let readout_h = if view.cursors.is_some() || !view.markers.is_empty() { 60.0 } else { 0.0 };
    let plot = Plot::new(("plot", name))
        .legend(Legend::default())
        .x_axis_label(l.xlabel.clone())
        .custom_y_axes(yaxes)
        .height(ui.available_height() - readout_h)
        .allow_drag(view.near.is_none() && view.drag.is_none());
    let auto = view.auto_fit;
    let mut near = None;
    let r = plot.show(ui, |pui| {
        if auto {
            let m = (primary.1 - primary.0) * 0.05;
            let (x0, x1) = if xr.1 > xr.0 { xr } else { (xr.0 - 0.5, xr.1 + 0.5) };
            pui.set_plot_bounds(PlotBounds::from_min_max([x0, primary.0 - m], [x1, primary.1 + m]));
        }
        let b = pui.plot_bounds();
        let width = pui.response().rect.width().max(100.0) as usize;
        let mut ci = 0;
        for (k, a) in l.axes.iter().enumerate() {
            let to = |y: f64| if k == 0 { y } else { map(y, ranges[k], primary) };
            for s in &a.series {
                let c = COLORS[ci % COLORS.len()];
                ci += 1;
                let label = if k == 0 { s.name.clone() } else { format!("{} (y{})", s.name, k + 1) };
                if s.is("infinite") {
                    for y in s.y.iter().rev().take(32) {
                        pui.hline(HLine::new(label.clone(), to(*y)).color(c));
                    }
                    continue;
                }
                let pts: Vec<[f64; 2]> =
                    visible(s, b.min()[0], b.max()[0], width * 2).into_iter().map(|[x, y]| [x, to(y)]).collect();
                if s.is("scatter") {
                    pui.points(Points::new(label, pts).color(c).radius(2.0));
                } else if s.is("digital") {
                    let mut st = Vec::with_capacity(pts.len() * 2);
                    for (i, p) in pts.iter().enumerate() {
                        if i > 0 {
                            st.push([p[0], pts[i - 1][1]]);
                        }
                        st.push(*p);
                    }
                    pui.line(Line::new(label, st).color(c));
                } else {
                    pui.line(Line::new(label, pts).color(c));
                }
            }
        }
        // Cursors: place them at a quarter and three quarters of the view the first time.
        if let Some(c) = &mut view.cursors {
            if c[0].is_nan() {
                let w = b.max()[0] - b.min()[0];
                *c = [b.min()[0] + w * 0.25, b.min()[0] + w * 0.75];
            }
            if let Some(ptr) = pui.pointer_coordinate() {
                let px = pui.screen_from_plot(ptr).x;
                near = c.iter().position(|cx| (pui.screen_from_plot(PlotPoint::new(*cx, ptr.y)).x - px).abs() < 6.0);
                let resp = pui.response().clone();
                if resp.drag_started() && near.is_some() {
                    view.drag = near;
                }
                if let Some(d) = view.drag {
                    c[d] = ptr.x;
                }
                if resp.drag_stopped() {
                    view.drag = None;
                }
            }
            for (i, x) in c.iter().enumerate() {
                pui.vline(
                    VLine::new(format!("cursor {}", i + 1), *x).color(Color32::from_rgb(240, 200, 60)).width(1.5),
                );
            }
        }
        for x in &view.markers {
            pui.vline(VLine::new("marker", *x).color(Color32::GRAY).style(LineStyle::dashed_dense()));
        }
        let resp = pui.response().clone();
        let cmd = resp.ctx.input(|i| i.modifiers.command);
        if resp.clicked() && cmd {
            if let Some(p) = pui.pointer_coordinate() {
                view.markers.push(p.x);
            }
        }
    });
    view.near = near;
    if view.drag.is_some() && !r.response.dragged() && !ui.input(|i| i.pointer.primary_down()) {
        view.drag = None;
    }
    // Panning or zooming hands the view to the user; double-click resets it.
    let zoomed =
        r.response.hovered() && ui.input(|i| i.smooth_scroll_delta != egui::Vec2::ZERO || i.zoom_delta() != 1.0);
    if (r.response.dragged() && view.drag.is_none() && view.near.is_none()) || zoomed {
        view.auto_fit = false;
    }
    if r.response.double_clicked() {
        view.auto_fit = true;
    }
    readout(ui, l, view);
}

fn readout(ui: &mut Ui, l: &LinePlot, view: &PlotView) {
    let series: Vec<(String, &Series)> = l
        .axes
        .iter()
        .enumerate()
        .flat_map(|(k, a)| {
            a.series.iter().map(move |s| (if k == 0 { s.name.clone() } else { format!("{} (y{})", s.name, k + 1) }, s))
        })
        .collect();
    egui::ScrollArea::vertical().id_salt("readout").max_height(56.0).show(ui, |ui| {
        if let Some([a, b]) = view.cursors {
            if !a.is_nan() {
                let dx = b - a;
                let mut t = format!("C1 x={}  C2 x={}  Δx={}", fmt(a), fmt(b), fmt(dx));
                if dx != 0.0 {
                    t += &format!("  1/Δx={}", fmt(1.0 / dx));
                }
                for (n, s) in &series {
                    if let (Some(ya), Some(yb)) = (value_at(s, a), value_at(s, b)) {
                        t += &format!("   {n}: {} → {} (Δ {})", fmt(ya), fmt(yb), fmt(yb - ya));
                    }
                }
                ui.label(RichText::new(t).monospace());
            }
        }
        for x in &view.markers {
            let mut t = format!("marker x={}", fmt(*x));
            for (n, s) in &series {
                if let Some(y) = value_at(s, *x) {
                    t += &format!("   {n}={}", fmt(y));
                }
            }
            ui.label(RichText::new(t).monospace());
        }
    });
}

/// Plasma-like palette (dark blue → yellow).
const PALETTE: [Color32; 5] = [
    Color32::from_rgb(13, 8, 135),
    Color32::from_rgb(126, 3, 168),
    Color32::from_rgb(204, 71, 120),
    Color32::from_rgb(248, 149, 64),
    Color32::from_rgb(240, 249, 33),
];

fn grid_plot(ui: &mut Ui, name: &str, g: &GridPlot, view: &mut PlotView) {
    let values = g.grid();
    let (lo, hi) = range(values.iter()).unwrap_or((0.0, 1.0));
    let filled: Vec<f64> = values.iter().map(|v| if v.is_finite() { *v } else { lo }).collect();
    // Spectrogram columns are labelled with their x; heatmap columns are numbered.
    let xs: Vec<f64> = if g.spectrogram {
        let skip = g.cols - g.columns.len().min(g.cols);
        (0..g.cols).map(|c| c.checked_sub(skip).and_then(|i| g.columns.get(i)).map_or(f64::NAN, |c| c.0)).collect()
    } else {
        vec![]
    };
    let mut plot = Plot::new(("grid", name))
        .x_axis_label(g.xlabel.clone())
        .y_axis_label(g.ylabel.clone())
        .show_grid(false)
        .allow_drag(true);
    if g.spectrogram {
        plot = plot.x_axis_formatter(move |m, _| {
            let i = m.value.floor();
            if i >= 0.0 && (i as usize) < xs.len() && xs[i as usize].is_finite() {
                fmt(xs[i as usize])
            } else {
                String::new()
            }
        });
    }
    let (rows, cols, auto) = (g.rows as f64, g.cols as f64, view.auto_fit);
    let r = plot.show(ui, |pui| {
        if auto {
            pui.set_plot_bounds(PlotBounds::from_min_max([0.0, 0.0], [cols, rows]));
        }
        let hi = if hi > lo { hi } else { lo + 1.0 };
        pui.heatmap(
            Heatmap::new(filled, g.cols)
                .palette(&PALETTE)
                .range(lo, hi)
                .show_labels(false)
                .at(PlotPoint::new(0.0, 0.0))
                .size(cols as f32, rows as f32)
                .name(name),
        );
    });
    let zoomed =
        r.response.hovered() && ui.input(|i| i.smooth_scroll_delta != egui::Vec2::ZERO || i.zoom_delta() != 1.0);
    if r.response.dragged() || zoomed {
        view.auto_fit = false;
    }
    if r.response.double_clicked() {
        view.auto_fit = true;
    }
}
