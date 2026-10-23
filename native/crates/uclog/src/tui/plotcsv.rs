//! `--plot-csv DIR`: port-60 plot data written as CSV, one file per plot (protocol.md §5.6).
//! The TUI does not draw plots.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use uclog_proto::cbor::{self, Value};

struct Plot {
    file: BufWriter<File>,
    /// Data keys in column order (`y11`, `y12`, … for line plots; `y11` for heatmaps).
    keys: Vec<String>,
    rows: u64,
}

pub struct PlotCsv {
    dir: PathBuf,
    plots: HashMap<String, Plot>,
    pub frames: u64,
    pub errors: u64,
}

fn texts(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_text().map(String::from)).collect(),
        _ => vec![],
    }
}

fn num(v: &Value) -> String {
    match v {
        Value::Integer(i) => i128::from(*i).to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => (*b as u8).to_string(),
        _ => String::new(),
    }
}

fn list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().map(num).collect(),
        Some(x) => vec![num(x)],
        None => vec![],
    }
}

fn safe(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect()
}

impl PlotCsv {
    pub fn new(dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(PlotCsv { dir, plots: HashMap::new(), frames: 0, errors: 0 })
    }

    fn create(&mut self, name: &str, keys: Vec<String>, header: Vec<String>) -> std::io::Result<()> {
        let path = self.dir.join(format!("{}.csv", safe(name)));
        let mut file = BufWriter::new(File::create(path)?);
        writeln!(file, "x,{}", header.join(","))?;
        file.flush()?;
        self.plots.insert(name.to_string(), Plot { file, keys, rows: 0 });
        Ok(())
    }

    /// One port-60 payload.
    pub fn handle(&mut self, data: &[u8]) {
        self.frames += 1;
        let Ok(m) = cbor::from_slice(data) else {
            self.errors += 1;
            return;
        };
        let r = self.handle_map(&m);
        if r.is_err() {
            self.errors += 1;
        }
    }

    fn handle_map(&mut self, m: &Value) -> std::io::Result<()> {
        let has = |k: &str| cbor::get(m, k).is_some();
        if let Some(name) = cbor::get_text(m, "plot") {
            if has("yser") {
                // Line plot: series of axes 1–3.
                let mut keys = vec![];
                let mut header = vec![];
                for (axis, key) in [(1, "yser"), (2, "y2ser"), (3, "y3ser")] {
                    for (i, s) in texts(cbor::get(m, key)).into_iter().enumerate() {
                        keys.push(format!("y{axis}{}", i + 1));
                        header.push(s);
                    }
                }
                return self.create(name, keys, header);
            }
            return self.data(name, m);
        }
        for kind in ["heatmap", "spectrogram"] {
            if let Some(name) = cbor::get_text(m, kind) {
                if has("heatmap_rows") || has("yser") || !self.plots.contains_key(name) {
                    let rows = cbor::get_int(m, "heatmap_rows").unwrap_or(0).max(0) as usize;
                    let header: Vec<String> = (0..rows).map(|i| format!("row{i}")).collect();
                    return self.create(name, vec!["y11".into()], header);
                }
                return self.data(name, m);
            }
        }
        if let Some(name) = cbor::get_text(m, "clear_plot") {
            if let Some(p) = self.plots.get_mut(name) {
                writeln!(p.file, "# cleared")?;
                p.file.flush()?;
            }
        }
        Ok(())
    }

    fn data(&mut self, name: &str, m: &Value) -> std::io::Result<()> {
        let Some(p) = self.plots.get_mut(name) else { return Ok(()) }; // created before we connected
        let cols: Vec<Vec<String>> = p.keys.iter().map(|k| list(cbor::get(m, k))).collect();
        let xs = list(cbor::get(m, "x"));
        if p.keys.len() == 1 && p.keys[0] == "y11" && cols[0].len() > 1 && xs.len() <= 1 {
            // Heatmap/spectrogram column: one row, x then the values.
            let x = xs.first().cloned().unwrap_or_else(|| p.rows.to_string());
            writeln!(p.file, "{x},{}", cols[0].join(","))?;
            p.rows += 1;
        } else {
            let n = cols.iter().map(Vec::len).max().unwrap_or(0).max(xs.len());
            for i in 0..n {
                let x = xs.get(i).cloned().unwrap_or_else(|| p.rows.to_string());
                let vals: Vec<&str> = cols.iter().map(|c| c.get(i).map(String::as_str).unwrap_or("")).collect();
                writeln!(p.file, "{x},{}", vals.join(","))?;
                p.rows += 1;
            }
        }
        p.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(v: serde_json::Value) -> Vec<u8> {
        fn conv(v: &serde_json::Value) -> Value {
            match v {
                serde_json::Value::String(s) => Value::Text(s.clone()),
                serde_json::Value::Number(n) if n.is_i64() => Value::Integer(n.as_i64().unwrap().into()),
                serde_json::Value::Number(n) => Value::Float(n.as_f64().unwrap()),
                serde_json::Value::Array(a) => Value::Array(a.iter().map(conv).collect()),
                serde_json::Value::Object(o) => {
                    Value::Map(o.iter().map(|(k, v)| (Value::Text(k.clone()), conv(v))).collect())
                }
                _ => Value::Null,
            }
        }
        cbor::to_vec(&conv(&v))
    }

    #[test]
    fn line_plot_and_heatmap() {
        let d = tempfile::tempdir().unwrap();
        let mut p = PlotCsv::new(d.path().into()).unwrap();
        p.handle(&enc(
            serde_json::json!({"plot": "adc", "xlabel": "t", "ylabel": "v", "yser": ["a", "b"], "y2ser": ["c"]}),
        ));
        p.handle(&enc(
            serde_json::json!({"plot": "adc", "x": [1, 2], "y11": [10, 11], "y12": [20, 21], "y21": [1.5, 2.5]}),
        ));
        p.handle(&enc(serde_json::json!({"plot": "adc", "y11": [12]})));
        p.handle(&enc(serde_json::json!({"plot": "unknown", "y11": [1]})));
        let t = std::fs::read_to_string(d.path().join("adc.csv")).unwrap();
        assert_eq!(t, "x,a,b,c\n1,10,20,1.5\n2,11,21,2.5\n2,12,,\n");
        p.handle(&enc(serde_json::json!({"heatmap": "fft", "heatmap_rows": 3, "heatmap_cols": 10, "xlabel": "t", "ylabel": "f", "yser": ["m"]})));
        p.handle(&enc(serde_json::json!({"heatmap": "fft", "x": 5, "y11": [1, 2, 3]})));
        assert_eq!(std::fs::read_to_string(d.path().join("fft.csv")).unwrap(), "x,row0,row1,row2\n5,1,2,3\n");
        assert_eq!(p.frames, 6);
    }
}
