//! Prints `file\taddr\ttext` for every format in the given .logdata files, using fixed
//! synthesized arguments (cross-check against spec/reference).
fn main() {
    for p in std::env::args().skip(1) {
        let ld = uclog_logdata::LogData::load(std::path::Path::new(&p)).unwrap();
        let mut addrs: Vec<_> = ld.fmts.keys().copied().collect();
        addrs.sort();
        for addr in addrs {
            let Some((_, kinds)) = &ld.fmts[&addr].fmt else { continue };
            let mut args = vec![];
            for k in kinds {
                use uclog_logdata::Kind::*;
                match k {
                    Int32 | Enum(_) => args.extend((-7i32).to_le_bytes()),
                    Uint32 | Pointer => args.extend(0xbeefu32.to_le_bytes()),
                    Sym => args.extend(0x2000_0104u32.to_le_bytes()),
                    Int64 => args.extend((-9i64).to_le_bytes()),
                    Uint64 => args.extend(0x1_0000_0001u64.to_le_bytes()),
                    Double => args.extend(1234.5678f64.to_le_bytes()),
                    String => args.extend(b"a\tb\0"),
                    Bytes => {}
                }
            }
            match ld.decode(addr, &args) {
                Ok(d) => println!("{p}\t{addr}\t{}", d.text),
                Err(e) => println!("{p}\t{addr}\tERR {}", e.kind()),
            }
        }
    }
}
