//! Log store for the Logs view: a bounded ring of records, filters, and the filtered index
//! (TUI plan §3, README §5.6).

use std::collections::{BTreeMap, VecDeque};

use regex::Regex;
use uclog_session::LogRecord;

pub const LEVELS: [&str; 6] = ["DEBUG", "INFO", "WARN", "ERROR", "FATAL", "PANIC"];

pub fn level_num(s: &str) -> u8 {
    LEVELS.iter().position(|l| l.eq_ignore_ascii_case(s.trim())).unwrap_or(3) as u8
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub seq: u64,
    pub ts: f64,
    pub level: u8,
    pub file: String,
    pub line: String,
    pub text: String,
    pub error: Option<&'static str>,
}

impl Row {
    pub fn from_record(seq: u64, r: &LogRecord) -> Row {
        Row {
            seq,
            ts: r.ts,
            level: level_num(&r.level),
            file: r.file.clone(),
            line: r.line.clone(),
            text: r.text.clone(),
            error: r.error,
        }
    }

    pub fn source(&self) -> String {
        if self.line.is_empty() {
            self.file.clone()
        } else {
            format!("{}:{}", self.file, self.line)
        }
    }

    /// `ts:level:file:line:text`, the legacy `uclog-cli.py` line format.
    pub fn plain(&self) -> String {
        format!("{:.3}:{}:{}:{}:{}", self.ts, LEVELS[self.level as usize], self.file, self.line, self.text)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub min_level: u8,
    /// Substring of the file name.
    pub source: String,
    /// Exact line number.
    pub line: String,
    pub search: Option<Regex>,
}

impl Filter {
    pub fn matches(&self, r: &Row) -> bool {
        // Error records are always shown (README §5.6).
        if r.error.is_some() {
            return self.search.as_ref().is_none_or(|re| re.is_match(&r.text));
        }
        r.level >= self.min_level
            && (self.source.is_empty() || r.file.contains(&self.source))
            && (self.line.is_empty() || r.line == self.line)
            && self.search.as_ref().is_none_or(|re| re.is_match(&r.text) || re.is_match(&r.file))
    }

    pub fn is_active(&self) -> bool {
        self.min_level > 0 || !self.source.is_empty() || !self.line.is_empty() || self.search.is_some()
    }

    pub fn describe(&self) -> String {
        let mut v = vec![format!("level ≥ {}", LEVELS[self.min_level as usize])];
        if !self.source.is_empty() {
            v.push(format!("source {}", self.source));
        }
        if !self.line.is_empty() {
            v.push(format!("line {}", self.line));
        }
        if let Some(re) = &self.search {
            v.push(format!("/{}/", re.as_str()));
        }
        v.join(" · ")
    }
}

pub struct LogStore {
    rows: VecDeque<Row>,
    max: usize,
    next_seq: u64,
    pub filter: Filter,
    /// Sequence numbers of the rows that pass the filter (ascending).
    visible: VecDeque<u64>,
    /// Drop rows that do not pass the filter instead of storing them.
    pub discard_filtered: bool,
    pub error_counts: BTreeMap<&'static str, u64>,
    pub received: u64,
    pub discarded: u64,
}

impl LogStore {
    pub fn new(max: usize) -> Self {
        LogStore {
            rows: VecDeque::new(),
            max: max.max(100),
            next_seq: 0,
            filter: Filter::default(),
            visible: VecDeque::new(),
            discard_filtered: false,
            error_counts: BTreeMap::new(),
            received: 0,
            discarded: 0,
        }
    }

    pub fn push(&mut self, r: &LogRecord) {
        self.received += 1;
        if let Some(k) = r.error {
            *self.error_counts.entry(k).or_insert(0) += 1;
        }
        let row = Row::from_record(self.next_seq, r);
        let ok = self.filter.matches(&row);
        if !ok && self.discard_filtered {
            self.discarded += 1;
            return;
        }
        self.next_seq += 1;
        if ok {
            self.visible.push_back(row.seq);
        }
        self.rows.push_back(row);
        while self.rows.len() > self.max {
            let old = self.rows.pop_front().unwrap();
            if self.visible.front() == Some(&old.seq) {
                self.visible.pop_front();
            }
        }
    }

    pub fn set_filter(&mut self, f: Filter) {
        self.filter = f;
        self.refilter();
    }

    fn refilter(&mut self) {
        self.visible = self.rows.iter().filter(|r| self.filter.matches(r)).map(|r| r.seq).collect();
    }

    pub fn set_max(&mut self, max: usize) {
        self.max = max.max(100);
        while self.rows.len() > self.max {
            self.rows.pop_front();
        }
        self.refilter();
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.visible.clear();
    }

    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// The `i`-th visible row.
    pub fn visible(&self, i: usize) -> Option<&Row> {
        let seq = *self.visible.get(i)?;
        let first = self.rows.front()?.seq;
        self.rows.get((seq - first) as usize)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn errors(&self) -> u64 {
        self.error_counts.values().sum()
    }

    /// CSV of the visible rows.
    pub fn csv(&self) -> String {
        let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut out = String::from("ts,level,file,line,text\n");
        for i in 0..self.visible_len() {
            let r = self.visible(i).unwrap();
            out.push_str(&format!(
                "{:.3},{},{},{},{}\n",
                r.ts,
                LEVELS[r.level as usize],
                q(&r.file),
                q(&r.line),
                q(&r.text)
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(level: &str, file: &str, text: &str) -> LogRecord {
        LogRecord {
            ts: 1.0,
            target: Some(0),
            level: level.into(),
            file: file.into(),
            line: "10".into(),
            text: text.into(),
            error: None,
        }
    }

    #[test]
    fn filters_and_ring() {
        let mut s = LogStore::new(100);
        for i in 0..150 {
            s.push(&rec(
                if i % 2 == 0 { "INFO" } else { "DEBUG" },
                if i % 3 == 0 { "a.c" } else { "b.c" },
                &format!("n={i}"),
            ));
        }
        assert_eq!(s.len(), 100);
        assert_eq!(s.visible_len(), 100);
        assert_eq!(s.visible(0).unwrap().text, "n=50");
        s.set_filter(Filter { min_level: 1, ..Default::default() });
        assert_eq!(s.visible_len(), 50);
        s.set_filter(Filter {
            source: "a.c".into(),
            search: Some(Regex::new("n=1[0-9]{2}$").unwrap()),
            ..Default::default()
        });
        assert!(s.visible(0).unwrap().text.starts_with("n=1"));
        assert!((0..s.visible_len()).all(|i| s.visible(i).unwrap().file == "a.c"));
        // Ring eviction keeps the visible index consistent.
        for i in 0..200 {
            s.push(&rec("INFO", "a.c", &format!("n={}", 150 + i)));
        }
        assert!((0..s.visible_len()).all(|i| s.visible(i).is_some()));
    }

    #[test]
    fn errors_always_shown_and_counted() {
        let mut s = LogStore::new(100);
        s.set_filter(Filter { min_level: 4, ..Default::default() });
        let mut e = rec("ERROR", "<uclog>", "decode: no logdata");
        e.error = Some("no_logdata");
        s.push(&e);
        s.push(&rec("INFO", "x.c", "hidden"));
        assert_eq!(s.visible_len(), 1);
        assert_eq!(s.errors(), 1);
        assert_eq!(s.error_counts["no_logdata"], 1);
    }

    #[test]
    fn discard_filtered_and_csv() {
        let mut s = LogStore::new(100);
        s.discard_filtered = true;
        s.set_filter(Filter { min_level: 2, ..Default::default() });
        s.push(&rec("INFO", "x.c", "dropped"));
        s.push(&rec("WARN", "x.c", "kept \"quoted\""));
        assert_eq!((s.len(), s.discarded), (1, 1));
        assert_eq!(s.csv(), "ts,level,file,line,text\n1.000,WARN,\"x.c\",\"10\",\"kept \"\"quoted\"\"\"\n");
        assert_eq!(s.visible(0).unwrap().plain(), "1.000:WARN:x.c:10:kept \"quoted\"");
    }
}
