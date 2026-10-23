//! Runs spec/vectors/*.json (see spec/reference/check_vectors.py).

use serde_json::Value as J;
use uclog_proto::semihost::{self, Request, TimeSample};
use uclog_proto::{cbor, cobs, crc32c::crc32c, fw, link, mux_decode, peek, FrameScanner, Mux, Peek, ScanEvent};

fn load(name: &str) -> J {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../spec/vectors").join(name);
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}

fn hx(v: &J) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

fn name(c: &J) -> &str {
    c["name"].as_str().unwrap_or("?")
}

fn scan_one(wire: &[u8]) -> Vec<u8> {
    let ev = FrameScanner::default().feed(wire);
    assert_eq!(ev.len(), 1, "{ev:?}");
    let ScanEvent::Frame(f) = &ev[0] else { panic!("{ev:?}") };
    cobs::decode(f).unwrap()
}

#[test]
fn cobs_vectors() {
    let v = load("cobs.json");
    for c in v["valid"].as_array().unwrap() {
        assert_eq!(cobs::encode(&hx(&c["decoded"])), hx(&c["encoded"]), "enc {}", name(c));
        assert_eq!(cobs::decode(&hx(&c["encoded"])).unwrap(), hx(&c["decoded"]), "dec {}", name(c));
    }
    for c in v["invalid"].as_array().unwrap() {
        assert!(cobs::decode(&hx(&c["encoded"])).is_err(), "invalid {}", name(c));
    }
}

#[test]
fn scanner_vectors() {
    for c in load("scanner.json")["cases"].as_array().unwrap() {
        let mut sc = FrameScanner::new(c["max_frame"].as_u64().unwrap() as usize);
        let mut ev = vec![];
        for ch in c["chunks"].as_array().unwrap() {
            ev.extend(sc.feed(&hx(ch)));
        }
        let want: Vec<ScanEvent> = c["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| match e.get("frame") {
                Some(f) => ScanEvent::Frame(hx(f)),
                None => ScanEvent::Oversize(e["oversize"].as_u64().unwrap() as usize),
            })
            .collect();
        assert_eq!(ev, want, "{}", name(c));
    }
}

#[test]
fn peek_mux_vectors() {
    for c in load("peek_mux.json")["cases"].as_array().unwrap() {
        let enc = hx(&c["encoded"]);
        let p = peek(&enc[..enc.len().min(2)]);
        let want = match c["peek"]["kind"].as_str().unwrap() {
            "pulse" => Peek::Pulse,
            "port" => Peek::Port(c["peek"]["port"].as_u64().unwrap() as u8),
            _ => Peek::Log(c["peek"]["type"].as_u64().unwrap() as u8),
        };
        // A 2-byte prefix of a longer frame must give the same answer as the whole frame.
        assert_eq!(peek(&enc), want, "peek {}", name(c));
        if enc.len() > 2 || enc != [1] {
            assert_eq!(p, want, "peek prefix {}", name(c));
        }
        let payload = cobs::decode(&enc).unwrap();
        assert_eq!(payload, hx(&c["payload"]));
        let m = &c["mux"];
        match (mux_decode(&payload), m["kind"].as_str().unwrap()) {
            (Mux::Pulse, "pulse") => {}
            (Mux::Port { port, data }, "port") => {
                assert_eq!(port as u64, m["port"].as_u64().unwrap(), "{}", name(c));
                assert_eq!(data, hx(&m["data"]), "{}", name(c));
            }
            (Mux::Log { ty, target, addr, args }, "log") => {
                assert_eq!(ty as u64, m["type"].as_u64().unwrap());
                assert_eq!(target as u64, m["target"].as_u64().unwrap());
                assert_eq!(addr as u64, m["addr"].as_u64().unwrap());
                assert_eq!(args, hx(&m["args"]));
            }
            (Mux::ShortLog(_), "error") => {}
            (got, want) => panic!("{}: got {got:?}, want {want}", name(c)),
        }
    }
}

fn json_to_cbor(m: &J) -> cbor::Value {
    match m {
        J::Object(o) if o.len() == 1 && o.contains_key("hex") => cbor::Value::Bytes(hx(&o["hex"])),
        J::Object(o) => {
            cbor::Value::Map(o.iter().map(|(k, v)| (cbor::Value::Text(k.clone()), json_to_cbor(v))).collect())
        }
        J::String(s) => cbor::Value::Text(s.clone()),
        J::Number(n) => cbor::Value::Integer(n.as_i64().unwrap().into()),
        J::Bool(b) => cbor::Value::Bool(*b),
        _ => unimplemented!("{m:?}"),
    }
}

#[test]
fn fw_update_vectors() {
    let v = load("fw_update.json");
    assert_eq!(crc32c(&hx(&v["crc32c_check"]["input"])) as u64, v["crc32c_check"]["crc"].as_u64().unwrap());
    let all = v["requests"].as_array().unwrap().iter().chain(v["replies"].as_array().unwrap());
    for c in all {
        let msg = json_to_cbor(&c["msg"]);
        assert_eq!(cbor::to_vec(&msg), hx(&c["cbor"]), "cbor {:?}", c["msg"]);
        for (key, port) in [("port59_wire", fw::FwPort::New), ("port3_wire", fw::FwPort::Legacy)] {
            let wire = hx(&c[key]);
            let payload = scan_one(&wire);
            let Mux::Port { port: p, data } = mux_decode(&payload) else { panic!() };
            assert_eq!(p, port.port());
            assert_eq!(fw::decode(port, data).unwrap(), msg, "{key} {:?}", c["msg"]);
            assert_eq!(uclog_proto::frame::wire_port(port.port(), &fw::encode(port, &msg)), wire, "{key} encode");
        }
    }
}

#[test]
fn semihost_vectors() {
    let v = load("semihost.json");
    for c in v["requests"].as_array().unwrap() {
        let req = hx(&c["request"]);
        let payload = scan_one(&hx(&c["wire"]));
        let Mux::Port { port: 63, data } = mux_decode(&payload) else { panic!() };
        assert_eq!(data, req);
        let r = semihost::parse_request(&req).unwrap();
        let f = &c["fields"];
        let op = c["op"].as_u64().unwrap() as u8;
        assert_eq!(semihost::op_name(op), c["op_name"].as_str(), "{}", name(c));
        assert_eq!(semihost::is_time_op(op), c["time_op"].as_bool().unwrap(), "{}", name(c));
        let i = |k: &str| f[k].as_i64().unwrap();
        let s = |k: &str| f[k].as_str().unwrap().to_string();
        let want = match op {
            2 => Request::Isatty { fid: i("fid") as i32 },
            6 => Request::Fstat { fid: i("fid") as i32 },
            7 => Request::Close { fid: i("fid") as i32 },
            8 => Request::Open { flags: i("flags") as i32, path: s("path") },
            9 => Request::Write { fid: i("fid") as u32, reply: f["reply"].as_bool().unwrap(), data: hx(&f["data"]) },
            11 => Request::Read { fid: i("fid") as i32, len: i("len") as u32 },
            12 => Request::Mkdir { mode: i("mode") as i32, path: s("path") },
            13 => Request::TimeOfDay,
            0x10 => Request::Clock,
            0x11 => Request::Time,
            0x30 => Request::Elapsed,
            0x31 => Request::TickFreq,
            _ => Request::Unknown { op },
        };
        assert_eq!(r, want, "{}", name(c));
    }
    for c in v["time_replies"].as_array().unwrap() {
        let inp = &c["inputs"];
        let t = match inp.get("qpc") {
            Some(q) => TimeSample {
                ticks: q.as_u64().unwrap(),
                freq: inp["qpf"].as_u64().unwrap(),
                wall: inp["wall"].as_f64().unwrap(),
            },
            None => TimeSample {
                ticks: inp["mono_ns"].as_u64().unwrap(),
                freq: 1_000_000_000,
                wall: inp["wall"].as_f64().unwrap(),
            },
        };
        let op = c["op"].as_u64().unwrap() as u8;
        assert_eq!(semihost::time_reply(op, &t).unwrap(), hx(&c["reply"]), "{} {}", c["source"], c["op_name"]);
    }
    for c in v["errors"].as_array().unwrap() {
        let e = semihost::errno::by_name(c["errno"].as_str().unwrap()).unwrap();
        assert_eq!(e as i64, c["value"].as_i64().unwrap());
        assert_eq!(semihost::error_reply(c["op"].as_u64().unwrap() as u8, e), hx(&c["reply"]), "{}", name(c));
    }
    for (k, val) in v["newlib_errno"].as_object().unwrap() {
        assert_eq!(semihost::errno::by_name(k).map(i64::from), val.as_i64(), "{k}");
    }
}

#[test]
fn link_vectors() {
    let v = load("links.json");
    for c in v["hid_out"].as_array().unwrap() {
        let got: Vec<Vec<u8>> = link::hid_reports(&hx(&c["data"])).iter().map(|r| r.to_vec()).collect();
        let want: Vec<Vec<u8>> = c["reports"].as_array().unwrap().iter().map(hx).collect();
        assert_eq!(got, want, "{}", name(c));
    }
    for c in v["stlink"].as_array().unwrap() {
        let d = hx(&c["data"]);
        let got: Vec<Vec<u8>> = link::stlink_split(&d).iter().map(|w| w.to_vec()).collect();
        let want: Vec<Vec<u8>> = c["writes"].as_array().unwrap().iter().map(hx).collect();
        assert_eq!(got, want, "{}", name(c));
    }
}

#[test]
fn device_info_vectors() {
    for c in load("device_info.json")["cases"].as_array().unwrap() {
        let p = scan_one(&hx(&c["wire"]));
        let Mux::Port { port: 62, data } = mux_decode(&p) else { panic!() };
        let info = cbor::from_slice(data).unwrap();
        assert_eq!(hex::encode(cbor::get_bytes(&info, "app_hash").unwrap()), c["app_hash_hex"].as_str().unwrap());
        assert_eq!(cbor::get_text(&info, "board"), c["board"].as_str());
    }
}
