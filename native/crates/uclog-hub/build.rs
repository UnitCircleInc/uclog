//! macOS: embed Info.plist so CoreBluetooth lets this command-line tool use BLE. All targets:
//! `UCLOG_VERSION`, the version as users see it.

fn main() {
    println!("cargo:rustc-env=UCLOG_VERSION={}", solover(env!("CARGO_PKG_VERSION")));
    let plist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/macos/Info-hub.plist");
    println!("cargo:rerun-if-changed={}", plist.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}", plist.display());
    }
}

/// The SoloVer form of the Cargo version (plan 08 §2.2a, scripts/version.py), shown to users:
/// 4.0.0 → 4, 4.0.0-rc.2 → 4-rc2, 4.0.3 → 4+hotfix3.
fn solover(v: &str) -> String {
    let (core, pre) = v.split_once('-').unwrap_or((v, ""));
    let mut parts = core.split('.');
    let (n, patch) = (parts.next().unwrap_or(v), parts.nth(1).unwrap_or("0"));
    if !pre.is_empty() {
        format!("{n}-{}", pre.replace('.', ""))
    } else if patch != "0" {
        format!("{n}+hotfix{patch}")
    } else {
        n.to_string()
    }
}
