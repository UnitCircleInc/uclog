//! Plot model (port 60, protocol.md §5.6): line plots with up to three y-axes, heatmaps and
//! spectrograms, as the legacy GUI drew them (`UnitPlotter.py`). No egui here: the view draws it.

use std::collections::VecDeque;

use uclog_proto::cbor::{self, Value};

/// Points kept per series when the plot sets no `xscroll`.
pub const SERIES_MAX: usize = 1_000_000;

#[derive(Debug, Clone, Default)]
pub struct Series {
    pub name: String,
    /// Renderer hints (`scatter`, `digital`, `infinite`).
    pub props: String,
    pub x: VecDeque<f64>,
    pub y: VecDeque<f64>,
}

impl Series {
    pub fn is(&self, prop: &str) -> bool {
        self.props.split(|c: char| !c.is_alphanumeric()).any(|p| p == prop)
    }
}

#[derive(Debug, Clone, Default)]
pub struct YAxis {
    pub label: String,
    pub limits: Option<(f64, f64)>,
    pub series: Vec<Series>,
}

#[derive(Debug, Clone, Default)]
pub struct LinePlot {
    pub xlabel: String,
    pub axes: Vec<YAxis>,
    /// Points kept per series (0 = all, up to [`SERIES_MAX`]).
    pub xscroll: usize,
}

#[derive(Debug, Clone, Default)]
pub struct GridPlot {
    pub spectrogram: bool,
    pub rows: usize,
    pub cols: usize,
    pub xlabel: String,
    pub ylabel: String,
    /// Heatmap: the latest grid, row-major, top row first.
    pub values: Vec<f64>,
    /// Spectrogram: the newest `cols` columns (x, one value per row, row 0 = lowest).
    pub columns: VecDeque<(f64, Vec<f64>)>,
}

impl GridPlot {
    /// Row-major values for drawing, top row first: the heatmap as sent; the spectrogram with the
    /// oldest column on the left and row 0 at the bottom. Missing cells are `NaN`.
    pub fn grid(&self) -> Vec<f64> {
        if !self.spectrogram {
            return self.values.clone();
        }
        let mut g = vec![f64::NAN; self.rows * self.cols];
        let skip = self.cols - self.columns.len().min(self.cols);
        for (c, (_, col)) in self.columns.iter().enumerate() {
            for (r, v) in col.iter().take(self.rows).enumerate() {
                g[(self.rows - 1 - r) * self.cols + skip + c] = *v;
            }
        }
        g
    }
}

#[derive(Debug, Clone)]
pub enum Kind {
    Line(LinePlot),
    Grid(GridPlot),
}

#[derive(Debug, Clone)]
pub struct Plot {
    pub name: String,
    pub kind: Kind,
    /// Bumped on every change.
    pub version: u64,
}

#[derive(Debug, Default)]
pub struct PlotSet {
    /// In creation order.
    pub plots: Vec<Plot>,
    pub frames: u64,
    /// Data for plots that do not exist (created before this client connected), counted.
    pub orphans: u64,
    pub errors: u64,
    /// Plots created since the view last looked (it opens a tab for each).
    pub created: Vec<String>,
}

fn texts(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().map(|x| x.as_text().map(String::from).unwrap_or_default()).collect(),
        Some(Value::Text(t)) => vec![t.clone()],
        _ => vec![],
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Integer(i) => Some(i128::from(*i) as f64),
        Value::Float(f) => Some(*f),
        Value::Bool(b) => Some(*b as u8 as f64),
        _ => None,
    }
}

fn nums(v: Option<&Value>) -> Vec<f64> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(num).collect(),
        Some(x) => num(x).into_iter().collect(),
        None => vec![],
    }
}

fn limits(v: Option<&Value>) -> Option<(f64, f64)> {
    let l = nums(v);
    (l.len() == 2 && l[1] > l[0]).then(|| (l[0], l[1]))
}

/// `yAS` → (axis 0–2, series 0–8).
fn series_key(k: &str) -> Option<(usize, usize)> {
    let b = k.as_bytes();
    if b.len() == 3 && b[0] == b'y' && (b'1'..=b'3').contains(&b[1]) && (b'1'..=b'9').contains(&b[2]) {
        Some(((b[1] - b'1') as usize, (b[2] - b'1') as usize))
    } else {
        None
    }
}

impl PlotSet {
    pub fn get(&self, name: &str) -> Option<&Plot> {
        self.plots.iter().find(|p| p.name == name)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut Plot> {
        self.plots.iter_mut().find(|p| p.name == name)
    }

    fn put(&mut self, name: &str, kind: Kind) {
        match self.get_mut(name) {
            Some(p) => {
                p.kind = kind;
                p.version += 1;
            }
            None => {
                self.plots.push(Plot { name: name.into(), kind, version: 0 });
                self.created.push(name.into());
            }
        }
    }

    /// One port-60 payload.
    pub fn handle(&mut self, data: &[u8]) {
        self.frames += 1;
        match cbor::from_slice(data) {
            Ok(m @ Value::Map(_)) => self.handle_map(&m),
            _ => self.errors += 1,
        }
    }

    fn handle_map(&mut self, m: &Value) {
        let has = |k: &str| cbor::get(m, k).is_some();
        if let Some(name) = cbor::get_text(m, "clear_plot") {
            let name = name.to_string();
            match self.get_mut(&name) {
                Some(p) => {
                    match &mut p.kind {
                        Kind::Line(l) => {
                            for s in l.axes.iter_mut().flat_map(|a| a.series.iter_mut()) {
                                s.x.clear();
                                s.y.clear();
                            }
                        }
                        Kind::Grid(g) => {
                            g.values.iter_mut().for_each(|v| *v = f64::NAN);
                            g.columns.clear();
                        }
                    }
                    p.version += 1;
                }
                None => self.orphans += 1,
            }
            return;
        }
        if let Some(name) = cbor::get_text(m, "plot") {
            let name = name.to_string();
            if has("xlabel") || has("yser") {
                // Create (or clear by re-creating).
                let ylabels = texts(cbor::get(m, "ylabel"));
                let mut axes = vec![];
                for (i, (ser, lim, props)) in
                    [("yser", "ylimits", "yprops"), ("y2ser", "y2limits", "y2props"), ("y3ser", "y3limits", "y3props")]
                        .into_iter()
                        .enumerate()
                {
                    let names = texts(cbor::get(m, ser));
                    if names.is_empty() {
                        if i == 0 {
                            continue;
                        }
                        break;
                    }
                    let props = texts(cbor::get(m, props));
                    axes.push(YAxis {
                        label: ylabels.get(i).cloned().unwrap_or_default(),
                        limits: limits(cbor::get(m, lim)),
                        series: names
                            .into_iter()
                            .enumerate()
                            .map(|(j, name)| Series {
                                name,
                                props: props.get(j).cloned().unwrap_or_default(),
                                ..Default::default()
                            })
                            .collect(),
                    });
                }
                let xscroll = cbor::get_int(m, "xscroll").unwrap_or(0).max(0) as usize;
                let xlabel = cbor::get_text(m, "xlabel").unwrap_or("").to_string();
                self.put(&name, Kind::Line(LinePlot { xlabel, axes, xscroll }));
                return;
            }
            let Some(Plot { kind: Kind::Line(l), version, .. }) = self.get_mut(&name) else {
                self.orphans += 1;
                return;
            };
            let xs = nums(cbor::get(m, "x"));
            let keep = if l.xscroll > 0 { l.xscroll.min(SERIES_MAX) } else { SERIES_MAX };
            for (k, v) in m.as_map().into_iter().flatten() {
                let Some((a, s)) = k.as_text().and_then(series_key) else { continue };
                let Some(series) = l.axes.get_mut(a).and_then(|ax| ax.series.get_mut(s)) else { continue };
                let ys = nums(Some(v));
                if series.is("infinite") {
                    series.y.extend(ys);
                } else {
                    for (i, y) in ys.into_iter().enumerate() {
                        // No x: number the samples.
                        let x = xs.get(i).copied().unwrap_or_else(|| series.x.back().map_or(0.0, |x| x + 1.0));
                        series.x.push_back(x);
                        series.y.push_back(y);
                    }
                }
                while series.y.len() > keep {
                    series.y.pop_front();
                }
                while series.x.len() > keep {
                    series.x.pop_front();
                }
            }
            *version += 1;
            return;
        }
        for (key, spectrogram) in [("heatmap", false), ("spectrogram", true)] {
            let Some(name) = cbor::get_text(m, key) else { continue };
            let name = name.to_string();
            if has("heatmap_rows") || has("xlabel") {
                let rows = cbor::get_int(m, "heatmap_rows").unwrap_or(0).clamp(0, 4096) as usize;
                let cols = cbor::get_int(m, "heatmap_cols").unwrap_or(0).clamp(0, 4096) as usize;
                if rows == 0 || cols == 0 {
                    self.errors += 1;
                    return;
                }
                let g = GridPlot {
                    spectrogram,
                    rows,
                    cols,
                    xlabel: cbor::get_text(m, "xlabel").unwrap_or("").to_string(),
                    ylabel: texts(cbor::get(m, "ylabel")).into_iter().next().unwrap_or_default(),
                    values: vec![f64::NAN; rows * cols],
                    columns: VecDeque::new(),
                };
                self.put(&name, Kind::Grid(g));
                return;
            }
            let Some(Plot { kind: Kind::Grid(g), version, .. }) = self.get_mut(&name) else {
                self.orphans += 1;
                return;
            };
            let v = nums(cbor::get(m, "y11"));
            if g.spectrogram {
                let x = nums(cbor::get(m, "x")).first().copied().unwrap_or(g.columns.len() as f64);
                g.columns.push_back((x, v));
                while g.columns.len() > g.cols {
                    g.columns.pop_front();
                }
            } else if v.len() == g.rows * g.cols {
                g.values = v;
            } else {
                self.errors += 1;
                return;
            }
            *version += 1;
            return;
        }
        self.errors += 1;
    }

    /// CSV of one plot: `x,<series>…` when the series share x values, else an `x_<name>,<name>`
    /// pair per series. Grids: one row per row (heatmap) or column (spectrogram, with its x).
    pub fn csv(&self, name: &str) -> Option<String> {
        let p = self.get(name)?;
        let q =
            |s: &str| if s.contains([',', '"']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
        let mut out = String::new();
        match &p.kind {
            Kind::Line(l) => {
                let all: Vec<&Series> = l.axes.iter().flat_map(|a| a.series.iter()).collect();
                let shared = all.windows(2).all(|w| w[0].x == w[1].x);
                if shared {
                    out.push_str(&q(if l.xlabel.is_empty() { "x" } else { &l.xlabel }).to_string());
                    for s in &all {
                        out.push_str(&format!(",{}", q(&s.name)));
                    }
                    out.push('\n');
                    let n = all.iter().map(|s| s.y.len()).max().unwrap_or(0);
                    for i in 0..n {
                        let x = all.first().and_then(|s| s.x.get(i)).map(|v| v.to_string()).unwrap_or_default();
                        out.push_str(&x);
                        for s in &all {
                            out.push(',');
                            if let Some(v) = s.y.get(i) {
                                out.push_str(&v.to_string());
                            }
                        }
                        out.push('\n');
                    }
                } else {
                    let head: Vec<String> = all.iter().map(|s| format!("x_{0},{0}", q(&s.name))).collect();
                    out.push_str(&head.join(","));
                    out.push('\n');
                    let n = all.iter().map(|s| s.y.len()).max().unwrap_or(0);
                    for i in 0..n {
                        let cells: Vec<String> = all
                            .iter()
                            .map(|s| {
                                let f = |v: Option<&f64>| v.map(|v| v.to_string()).unwrap_or_default();
                                format!("{},{}", f(s.x.get(i)), f(s.y.get(i)))
                            })
                            .collect();
                        out.push_str(&cells.join(","));
                        out.push('\n');
                    }
                }
            }
            Kind::Grid(g) if g.spectrogram => {
                out.push('x');
                for r in 0..g.rows {
                    out.push_str(&format!(",row{r}"));
                }
                out.push('\n');
                for (x, col) in &g.columns {
                    out.push_str(&x.to_string());
                    for v in col {
                        out.push_str(&format!(",{v}"));
                    }
                    out.push('\n');
                }
            }
            Kind::Grid(g) => {
                for r in 0..g.rows {
                    let row: Vec<String> =
                        g.values[r * g.cols..(r + 1) * g.cols].iter().map(|v| v.to_string()).collect();
                    out.push_str(&row.join(","));
                    out.push('\n');
                }
            }
        }
        Some(out)
    }
}

/// Data range of the finite values, or `None`.
pub fn range<'a>(v: impl IntoIterator<Item = &'a f64>) -> Option<(f64, f64)> {
    let mut r: Option<(f64, f64)> = None;
    for &x in v {
        if x.is_finite() {
            r = Some(match r {
                Some((a, b)) => (a.min(x), b.max(x)),
                None => (x, x),
            });
        }
    }
    r
}

/// Points of a series with x in `[x0, x1]` (plus one on each side, so lines reach the edges),
/// decimated to at most about `max` points by keeping the min and max of each bucket.
pub fn visible(s: &Series, x0: f64, x1: f64, max: usize) -> Vec<[f64; 2]> {
    let n = s.x.len().min(s.y.len());
    // Series are usually in x order; then a binary search finds the window.
    let sorted = s.x.iter().zip(s.x.iter().skip(1)).all(|(a, b)| a <= b);
    let (lo, hi) = if sorted {
        let lo = s.x.partition_point(|&x| x < x0).saturating_sub(1);
        let hi = (s.x.partition_point(|&x| x <= x1) + 1).min(n);
        (lo, hi)
    } else {
        (0, n)
    };
    let pts: Vec<[f64; 2]> =
        (lo..hi).filter(|&i| sorted || (s.x[i] >= x0 && s.x[i] <= x1)).map(|i| [s.x[i], s.y[i]]).collect();
    if max < 4 || pts.len() <= max {
        return pts;
    }
    let buckets = max / 2;
    let per = pts.len().div_ceil(buckets);
    let mut out = Vec::with_capacity(buckets * 2);
    for chunk in pts.chunks(per) {
        let (mut a, mut b) = (chunk[0], chunk[0]);
        for p in chunk {
            if p[1] < a[1] {
                a = *p;
            }
            if p[1] > b[1] {
                b = *p;
            }
        }
        if a[0] <= b[0] {
            out.push(a);
            if b != a {
                out.push(b);
            }
        } else {
            out.push(b);
            out.push(a);
        }
    }
    // Keep the ends, so lines reach the edges of the view.
    if out.first() != pts.first() {
        out.insert(0, pts[0]);
    }
    if out.last() != pts.last() {
        out.push(*pts.last().unwrap());
    }
    out
}

/// Map `v` from range `a` onto range `p` (secondary y-axes share the primary axis' coordinates).
pub fn map(v: f64, a: (f64, f64), p: (f64, f64)) -> f64 {
    let span = a.1 - a.0;
    if span == 0.0 {
        return (p.0 + p.1) / 2.0;
    }
    p.0 + (v - a.0) / span * (p.1 - p.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uclog_proto::cbor::map as m;
    use Value::{Array, Float, Integer, Text};

    fn enc(v: Value) -> Vec<u8> {
        cbor::to_vec(&v)
    }
    fn fl(v: &[f64]) -> Value {
        Array(v.iter().map(|x| Float(*x)).collect())
    }
    fn tx(v: &[&str]) -> Value {
        Array(v.iter().map(|x| Text((*x).into())).collect())
    }

    #[test]
    fn line_plot_axes_scroll_and_csv() {
        let mut p = PlotSet::default();
        p.handle(&enc(m([("plot", Text("adc".into())), ("y11", fl(&[1.0]))])));
        assert_eq!(p.orphans, 1);
        p.handle(&enc(m([
            ("plot", Text("adc".into())),
            ("xlabel", Text("t".into())),
            ("ylabel", tx(&["V", "%"])),
            ("yser", tx(&["a", "b"])),
            ("y2ser", tx(&["c"])),
            ("y2limits", fl(&[0.0, 100.0])),
            ("yprops", tx(&["", "scatter"])),
            ("xscroll", Integer(3.into())),
        ])));
        assert_eq!(p.created, vec!["adc"]);
        p.handle(&enc(m([
            ("plot", Text("adc".into())),
            ("x", fl(&[1.0, 2.0])),
            ("y11", fl(&[10.0, 11.0])),
            ("y12", fl(&[20.0, 21.0])),
            ("y21", fl(&[50.0, 60.0])),
        ])));
        p.handle(&enc(m([("plot", Text("adc".into())), ("x", fl(&[3.0, 4.0])), ("y11", fl(&[12.0, 13.0]))])));
        let Kind::Line(l) = &p.get("adc").unwrap().kind else { panic!() };
        assert_eq!(l.axes.len(), 2);
        assert_eq!((l.axes[1].label.as_str(), l.axes[1].limits), ("%", Some((0.0, 100.0))));
        assert!(l.axes[0].series[1].is("scatter"));
        assert_eq!(l.axes[0].series[0].y, [11.0, 12.0, 13.0], "xscroll keeps 3");
        assert_eq!(l.axes[0].series[0].x, [2.0, 3.0, 4.0]);
        assert_eq!(p.csv("adc").unwrap().lines().next().unwrap(), "x_a,a,x_b,b,x_c,c");
        // Re-creating clears.
        p.handle(&enc(m([("plot", Text("adc".into())), ("xlabel", Text("t".into())), ("yser", tx(&["a"]))])));
        let Kind::Line(l) = &p.get("adc").unwrap().kind else { panic!() };
        assert!(l.axes[0].series[0].y.is_empty());
        assert_eq!(p.created.len(), 1);
        p.handle(&enc(m([("plot", Text("adc".into())), ("y11", fl(&[5.0, 6.0]))])));
        assert_eq!(p.csv("adc").unwrap(), "t,a\n0,5\n1,6\n");
    }

    #[test]
    fn heatmap_spectrogram_clear() {
        let mut p = PlotSet::default();
        let create = |k: &'static str, rows: i64, cols: i64| {
            enc(m([
                (k, Text("g".into())),
                ("heatmap_rows", Integer(rows.into())),
                ("heatmap_cols", Integer(cols.into())),
                ("xlabel", Text("t".into())),
                ("ylabel", tx(&["f"])),
                ("yser", tx(&["p"])),
            ]))
        };
        p.handle(&create("heatmap", 2, 2));
        p.handle(&enc(m([("heatmap", Text("g".into())), ("x", Float(0.0)), ("y11", fl(&[1.0, 2.0, 3.0, 4.0]))])));
        let Kind::Grid(g) = &p.get("g").unwrap().kind else { panic!() };
        assert_eq!(g.grid(), vec![1.0, 2.0, 3.0, 4.0]);
        p.handle(&enc(m([("heatmap", Text("g".into())), ("y11", fl(&[1.0]))])));
        assert_eq!(p.errors, 1, "wrong size");

        p.handle(&create("spectrogram", 2, 3));
        for i in 0..4 {
            let v = i as f64;
            p.handle(&enc(m([("spectrogram", Text("g".into())), ("x", Float(v)), ("y11", fl(&[v, v + 10.0]))])));
        }
        let Kind::Grid(g) = &p.get("g").unwrap().kind else { panic!() };
        // Columns 1..3 kept; row 1 (high) on top.
        assert_eq!(g.grid(), vec![11.0, 12.0, 13.0, 1.0, 2.0, 3.0]);
        assert_eq!(p.csv("g").unwrap().lines().nth(1).unwrap(), "1,1,11");
        p.handle(&enc(m([("clear_plot", Text("g".into()))])));
        let Kind::Grid(g) = &p.get("g").unwrap().kind else { panic!() };
        assert!(g.grid().iter().all(|v| v.is_nan()));
    }

    #[test]
    fn decimation_and_mapping() {
        let s = Series {
            x: (0..10_000).map(|i| i as f64).collect(),
            y: (0..10_000).map(|i| if i == 5000 { 100.0 } else { (i % 7) as f64 }).collect(),
            ..Default::default()
        };
        let v = visible(&s, 1000.0, 9000.0, 200);
        assert!(v.len() <= 200 && v.len() > 100, "{}", v.len());
        assert!(v.iter().any(|p| p[1] == 100.0), "the peak survives decimation");
        assert!(v.first().unwrap()[0] <= 1000.0 && v.last().unwrap()[0] >= 9000.0);
        assert_eq!(visible(&s, 10.0, 12.0, 200).len(), 5);
        assert_eq!(map(50.0, (0.0, 100.0), (-1.0, 1.0)), 0.0);
        assert_eq!(range(&[f64::NAN, 3.0, -1.0]), Some((-1.0, 3.0)));
    }
}
