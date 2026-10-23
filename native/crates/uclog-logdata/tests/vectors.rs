//! Runs spec/vectors/logs.json against sample.logdata.

use serde_json::Value as J;
use uclog_logdata::LogData;
use uclog_proto::{cobs, mux_decode, FrameScanner, Mux, ScanEvent};

fn vectors() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../spec/vectors")
}

fn log_frame(wire: &str) -> (u8, u32, Vec<u8>) {
    let ev = FrameScanner::default().feed(&hex::decode(wire).unwrap());
    let [ScanEvent::Frame(f)] = &ev[..] else { panic!("{ev:?}") };
    let p = cobs::decode(f).unwrap();
    let Mux::Log { target, addr, args, .. } = mux_decode(&p) else { panic!() };
    (target, addr, args.to_vec())
}

#[test]
fn logs_vectors() {
    let v: J = serde_json::from_str(&std::fs::read_to_string(vectors().join("logs.json")).unwrap()).unwrap();
    let ld = LogData::load(&vectors().join(v["logdata"].as_str().unwrap())).unwrap();
    assert_eq!(ld.target() as u64, v["logdata_target"].as_u64().unwrap());
    for c in v["cases"].as_array().unwrap() {
        let (target, addr, args) = log_frame(c["wire"].as_str().unwrap());
        assert_eq!(target as u64, c["target"].as_u64().unwrap());
        let d = ld.decode(addr, &args).unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        let e = &c["expect"];
        assert_eq!(
            (d.level.as_str(), d.file.as_str(), d.line.as_str(), d.text.as_str()),
            (
                e["level"].as_str().unwrap(),
                e["file"].as_str().unwrap(),
                e["line"].as_str().unwrap(),
                e["text"].as_str().unwrap()
            ),
            "{}",
            c["name"]
        );
    }
    for c in v["errors"].as_array().unwrap() {
        let (_, addr, args) = log_frame(c["wire"].as_str().unwrap());
        let e = ld.decode(addr, &args).unwrap_err();
        assert_eq!(e.kind(), c["error"].as_str().unwrap(), "{}", c["name"]);
    }
    let c = &v["no_logdata_for_target"];
    let (target, _, _) = log_frame(c["wire"].as_str().unwrap());
    assert_ne!(target, ld.target());
}

/// Decodes every format in local .logdata files with synthesized arguments (run with
/// `UCLOG_LOGDATA_CORPUS=~/.cache/uclog cargo test -- --ignored`).
#[test]
#[ignore]
fn corpus() {
    let Ok(dir) = std::env::var("UCLOG_LOGDATA_CORPUS") else { return };
    let (mut ok, mut files) = (0, 0);
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().and_then(|x| x.to_str()) != Some("logdata") {
            continue;
        }
        let ld = LogData::load(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        files += 1;
        for (addr, f) in &ld.fmts {
            let Some((_, kinds)) = &f.fmt else { continue };
            let mut args = vec![];
            for k in kinds {
                use uclog_logdata::Kind::*;
                match k {
                    Int32 | Uint32 | Pointer | Sym | Enum(_) => args.extend(7u32.to_le_bytes()),
                    Int64 | Uint64 => args.extend(9u64.to_le_bytes()),
                    Double => args.extend(1.5f64.to_le_bytes()),
                    String => args.extend(b"hi\0"),
                    Bytes => {}
                }
            }
            ld.decode(*addr, &args).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            ok += 1;
        }
    }
    eprintln!("decoded {ok} formats from {files} files");
}
