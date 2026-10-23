//! `.logdata` loading and log-frame decoding (protocol.md §6).
//!
//! Pure code apart from the file helpers ([`LogData::load`]); WASM-friendly otherwise.

pub mod printf;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use ciborium::Value;
use printf::Arg;
use uclog_proto::frame::{TARGET_DIGIT_SHIFT, TYPE_MEM};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("invalid CBOR: {0}")]
    Cbor(String),
    #[error("invalid logdata: {0}")]
    Format(String),
    #[error("ELF: {0}")]
    Elf(String),
    #[error("no .logdata_cbor section in ELF file")]
    NoSection,
}

/// Argument kind from the logdata `kinds` list (protocol.md §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Int32,
    Uint32,
    Int64,
    Uint64,
    Double,
    Pointer,
    String,
    Sym,
    Bytes,
    Enum(String),
}

impl Kind {
    fn name(&self) -> &str {
        match self {
            Kind::Int32 => "int32",
            Kind::Uint32 => "uint32",
            Kind::Int64 => "int64",
            Kind::Uint64 => "uint64",
            Kind::Double => "double",
            Kind::Pointer => "pointer",
            Kind::String => "string",
            Kind::Sym => "sym",
            Kind::Bytes => "bytes",
            Kind::Enum(_) => "enum",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Format {
    pub level: String,
    pub file: String,
    pub line: String,
    /// `None` for `[level, file, line]` entries (no format string).
    pub fmt: Option<(String, Vec<Kind>)>,
}

/// One loaded `.logdata` file.
#[derive(Debug, Clone)]
pub struct LogData {
    pub saddr: u32,
    pub fmts: HashMap<u32, Format>,
    pub enums: HashMap<String, BTreeMap<i64, String>>,
    pub tdenums: HashMap<String, BTreeMap<i64, String>>,
    /// (low, size, name), sorted by low.
    pub fns: Vec<(u32, u32, String)>,
    /// address → `file:name`.
    pub vars: BTreeMap<u32, String>,
    /// App hash from a `<hash>.logdata` file name, if any.
    pub app_hash: Option<Vec<u8>>,
    /// Where it was loaded from (for display).
    pub source: String,
}

/// A decoded log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub level: String,
    pub file: String,
    pub line: String,
    pub text: String,
}

/// Decode failure kinds (protocol.md §7).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("unknown format addr 0x{addr:08x}")]
    UnknownFormat { addr: u32 },
    #[error("{file}:{line} \"{fmt}\" — missing arg {index} ({kind})")]
    MissingArg { file: String, line: String, fmt: String, index: usize, kind: String },
    #[error("{file}:{line} \"{fmt}\" — extra {n} bytes: {hex}")]
    ExtraBytes { file: String, line: String, fmt: String, n: usize, hex: String },
    #[error("{file}:{line} \"{fmt}\" — {msg}")]
    Format { file: String, line: String, fmt: String, msg: String },
}

impl DecodeError {
    /// Kind name used by the vectors and error counters.
    pub fn kind(&self) -> &'static str {
        match self {
            DecodeError::UnknownFormat { .. } => "unknown_format",
            DecodeError::MissingArg { .. } => "missing_arg",
            DecodeError::ExtraBytes { .. } => "extra_bytes",
            DecodeError::Format { .. } => "format",
        }
    }
}

const LEVELS: [&str; 6] = ["DEBUG", "INFO", "WARN", "ERROR", "FATAL", "PANIC"];

fn err(m: impl Into<String>) -> LoadError {
    LoadError::Format(m.into())
}

fn text(v: &Value) -> Result<String, LoadError> {
    v.as_text().map(str::to_string).ok_or_else(|| err(format!("expected text, got {v:?}")))
}

fn int(v: &Value) -> Result<i128, LoadError> {
    v.as_integer().map(i128::from).ok_or_else(|| err(format!("expected integer, got {v:?}")))
}

fn enum_table(v: &Value) -> Result<HashMap<String, BTreeMap<i64, String>>, LoadError> {
    let mut out = HashMap::new();
    for (k, m) in v.as_map().ok_or_else(|| err("enums: expected map"))? {
        let mut t = BTreeMap::new();
        for (val, name) in m.as_map().ok_or_else(|| err("enum: expected map"))? {
            t.insert(int(val)? as i64, text(name)?);
        }
        out.insert(text(k)?, t);
    }
    Ok(out)
}

fn kind(v: &Value) -> Result<Kind, LoadError> {
    if let Some(a) = v.as_array() {
        if a.len() == 2 && a[0].as_text() == Some("enum") {
            return Ok(Kind::Enum(text(&a[1])?));
        }
        return Err(err(format!("bad kind {v:?}")));
    }
    Ok(match text(v)?.as_str() {
        "int32" => Kind::Int32,
        "uint32" => Kind::Uint32,
        "int64" => Kind::Int64,
        "uint64" => Kind::Uint64,
        "double" => Kind::Double,
        "pointer" => Kind::Pointer,
        "string" => Kind::String,
        "sym" => Kind::Sym,
        "bytes" => Kind::Bytes,
        k => return Err(err(format!("unknown kind {k:?}"))),
    })
}

impl LogData {
    /// Parse `.logdata` CBOR.
    pub fn from_cbor(data: &[u8], source: impl Into<String>) -> Result<Self, LoadError> {
        let v: Value = ciborium::from_reader(data).map_err(|e| LoadError::Cbor(e.to_string()))?;
        let get = |k: &str| uclog_proto::cbor::get(&v, k).ok_or_else(|| err(format!("missing key {k:?}")));
        let mut fmts = HashMap::new();
        for (a, e) in get("fmts")?.as_map().ok_or_else(|| err("fmts: expected map"))? {
            let e = e.as_array().ok_or_else(|| err("fmt entry: expected array"))?;
            if e.len() != 5 && e.len() != 3 {
                return Err(err(format!("fmt entry with {} fields", e.len())));
            }
            let fmt = if e.len() == 5 {
                let kinds = e[4]
                    .as_array()
                    .ok_or_else(|| err("kinds: expected array"))?
                    .iter()
                    .map(kind)
                    .collect::<Result<_, _>>()?;
                Some((text(&e[3])?, kinds))
            } else {
                None
            };
            fmts.insert(int(a)? as u32, Format { level: text(&e[0])?, file: text(&e[1])?, line: text(&e[2])?, fmt });
        }
        let mut fns = Vec::new();
        for (k, name) in get("fns")?.as_map().ok_or_else(|| err("fns: expected map"))? {
            let k = k.as_array().filter(|a| a.len() == 2).ok_or_else(|| err("fns key: expected [low, size]"))?;
            fns.push((int(&k[0])? as u32, int(&k[1])? as u32, text(name)?));
        }
        fns.sort();
        let mut vars = BTreeMap::new();
        for (a, name) in get("vars")?.as_map().ok_or_else(|| err("vars: expected map"))? {
            vars.insert(int(a)? as u32, text(name)?);
        }
        Ok(LogData {
            saddr: int(get("saddr")?)? as u32,
            fmts,
            enums: enum_table(get("enums")?)?,
            tdenums: enum_table(get("tdenums")?)?,
            fns,
            vars,
            app_hash: None,
            source: source.into(),
        })
    }

    /// Extract the `.logdata_cbor` section of an ELF file.
    pub fn cbor_from_elf(data: &[u8]) -> Result<Vec<u8>, LoadError> {
        use object::{Object, ObjectSection};
        let f = object::File::parse(data).map_err(|e| LoadError::Elf(e.to_string()))?;
        let s = f.section_by_name(".logdata_cbor").ok_or(LoadError::NoSection)?;
        let d = s.data().map_err(|e| LoadError::Elf(e.to_string()))?;
        if d.is_empty() {
            return Err(LoadError::NoSection);
        }
        Ok(d.to_vec())
    }

    /// Load a `.logdata`/`.cbor` file or an ELF file with `.logdata_cbor`.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        let data = std::fs::read(path)?;
        let src = path.display().to_string();
        let mut ld = if data.starts_with(b"\x7fELF") {
            Self::from_cbor(&Self::cbor_from_elf(&data)?, src)?
        } else {
            Self::from_cbor(&data, src)?
        };
        let stem = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.split('.').next()).unwrap_or("");
        if stem.len() == 128 {
            ld.app_hash = decode_hex(stem);
        }
        Ok(ld)
    }

    /// Target digit this logdata decodes.
    pub fn target(&self) -> u8 {
        ((self.saddr >> TARGET_DIGIT_SHIFT) & 0xF) as u8
    }

    pub fn lookup_sym(&self, a: u32) -> String {
        let a2 = a & !1;
        let i = self.fns.partition_point(|(low, _, _)| *low <= a2);
        if let Some((low, size, name)) = i.checked_sub(1).map(|i| &self.fns[i]) {
            if a2 < low.wrapping_add(*size) {
                return format!("{name}+0x{:x}", a2 - low);
            }
        }
        // Fall back to a linear scan in case of overlapping ranges.
        for (low, size, name) in &self.fns {
            if *low <= a2 && a2 < low.wrapping_add(*size) {
                return format!("{name}+0x{:x}", a2 - low);
            }
        }
        if let Some((addr, name)) = self.vars.range(..=a).next_back() {
            if a - addr < 0x3000 {
                return format!("{name}+0x{:x}", a - addr);
            }
        }
        format!("0x{a:08x}")
    }

    pub fn enum_name(&self, name: &str, v: i32) -> String {
        if let Some(t) = self.enums.get(name) {
            return t.get(&(v as i64)).cloned().unwrap_or_else(|| format!("<{name}:{v}>"));
        }
        if let Some(t) = self.tdenums.get(name) {
            return t.get(&(v as i64)).cloned().unwrap_or_else(|| format!("<{name}:{v}>"));
        }
        format!("<!{name}:{v}>")
    }

    /// Decode a type-0/1 log frame.
    pub fn decode(&self, addr: u32, args: &[u8]) -> Result<Decoded, DecodeError> {
        let key = addr & !3;
        let f = self.fmts.get(&key).ok_or(DecodeError::UnknownFormat { addr: key })?;
        let Some((fmt, kinds)) = &f.fmt else { return Err(DecodeError::UnknownFormat { addr: key }) };
        let level = match f.level.parse::<usize>().ok().and_then(|l| LEVELS.get(l)) {
            Some(l) => l.to_string(),
            None => format!("<bad level {}>", f.level),
        };
        let mut line = f.line.clone();
        let missing = |index: usize, kind: &str| DecodeError::MissingArg {
            file: f.file.clone(),
            line: f.line.clone(),
            fmt: fmt.clone(),
            index,
            kind: kind.to_string(),
        };
        let text = if (addr & 3) as u8 == TYPE_MEM {
            if args.len() < 4 {
                return Err(missing(0, "uint32"));
            }
            let ptr = u32::from_le_bytes(args[..4].try_into().unwrap());
            let rest = &args[4..];
            if f.line == "<zephyr>" {
                line.clear();
                String::from_utf8_lossy(rest).into_owned()
            } else {
                let hex: Vec<String> = rest.iter().map(|b| format!("{b:02x}")).collect();
                format!("{fmt} {ptr:08x}: {}", hex.join(" "))
            }
        } else {
            let mut vals = Vec::with_capacity(kinds.len());
            let mut rest = args;
            for (i, k) in kinds.iter().enumerate() {
                let take = |n: usize, rest: &mut &[u8]| -> Result<Vec<u8>, DecodeError> {
                    if rest.len() < n {
                        return Err(missing(i, k.name()));
                    }
                    let (a, b) = rest.split_at(n);
                    *rest = b;
                    Ok(a.to_vec())
                };
                let v = match k {
                    Kind::Int32 => {
                        Arg::Int { v: i32::from_le_bytes(take(4, &mut rest)?.try_into().unwrap()) as i128, size: 4 }
                    }
                    Kind::Uint32 | Kind::Pointer => {
                        Arg::Int { v: u32::from_le_bytes(take(4, &mut rest)?.try_into().unwrap()) as i128, size: 4 }
                    }
                    Kind::Int64 => {
                        Arg::Int { v: i64::from_le_bytes(take(8, &mut rest)?.try_into().unwrap()) as i128, size: 8 }
                    }
                    Kind::Uint64 => {
                        Arg::Int { v: u64::from_le_bytes(take(8, &mut rest)?.try_into().unwrap()) as i128, size: 8 }
                    }
                    Kind::Double => Arg::Float(f64::from_le_bytes(take(8, &mut rest)?.try_into().unwrap())),
                    Kind::Sym => Arg::Str(self.lookup_sym(u32::from_le_bytes(take(4, &mut rest)?.try_into().unwrap()))),
                    Kind::Enum(n) => {
                        Arg::Str(self.enum_name(n, i32::from_le_bytes(take(4, &mut rest)?.try_into().unwrap())))
                    }
                    Kind::String => {
                        let z = rest.iter().position(|&b| b == 0).ok_or_else(|| missing(i, "string"))?;
                        let s = escape_str(&String::from_utf8_lossy(&rest[..z]));
                        rest = &rest[z + 1..];
                        Arg::Str(s)
                    }
                    Kind::Bytes => {
                        let s = rest.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");
                        rest = &[];
                        Arg::Str(s)
                    }
                };
                vals.push(v);
            }
            if !rest.is_empty() {
                return Err(DecodeError::ExtraBytes {
                    file: f.file.clone(),
                    line: f.line.clone(),
                    fmt: fmt.clone(),
                    n: rest.len(),
                    hex: rest.iter().map(|b| format!("{b:02x}")).collect(),
                });
            }
            printf::format(fmt, &vals).map_err(|e| DecodeError::Format {
                file: f.file.clone(),
                line: f.line.clone(),
                fmt: fmt.clone(),
                msg: e.to_string(),
            })?
        };
        Ok(Decoded { level, file: f.file.clone(), line, text })
    }
}

/// Escape a `%s` argument (protocol.md §6.3).
pub fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7F => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// Logdata for several targets (one per target digit).
#[derive(Debug, Default, Clone)]
pub struct Decoders {
    by_target: HashMap<u8, std::sync::Arc<LogData>>,
}

impl Decoders {
    pub fn insert(&mut self, ld: std::sync::Arc<LogData>) {
        self.by_target.insert(ld.target(), ld);
    }

    pub fn get(&self, target: u8) -> Option<&std::sync::Arc<LogData>> {
        self.by_target.get(&target)
    }

    pub fn clear(&mut self) {
        self.by_target.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.by_target.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &std::sync::Arc<LogData>> {
        self.by_target.values()
    }
}
