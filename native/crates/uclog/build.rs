//! macOS: embed Info.plist so CoreBluetooth lets this command-line tool use BLE.

fn main() {
    let plist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/macos/Info.plist");
    println!("cargo:rerun-if-changed={}", plist.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}", plist.display());
    }
}
