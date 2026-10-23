# Native workspace (Rust)

Phase 1 of [docs/plans](../docs/plans/README.md): the shared crates and the first `uclog`
subcommands. `gui`, `tui` and `hub` come in later phases.

| Crate | Contents |
|---|---|
| `uclog-proto` | COBS, frame scanner, 2-byte peek, mux, CBOR helpers, CRC-32C, FW-update messages, semihost codec, link chunking. No I/O (WASM-friendly) |
| `uclog-logdata` | `.logdata` / ELF `.logdata_cbor` loader, log decoding, C-printf formatter |
| `uclog-semihost` | sandboxed port-63 handler, newlib errno mapping, host clock sources (QPC / `CLOCK_MONOTONIC`) |
| `uclog-link` | Serial, HID, BLE, UDP, TCP and WebSocket links; device URIs; USB/HID/BLE/mDNS discovery |
| `uclog-session` | pulses and wake gap, liveness, dispatch, logdata resolver, semihost bridge, FW update (59 → 3) |
| `uclog` | the executable: `list`, `decode`, `term`, `fw-update` |

```sh
cargo test                       # unit tests, spec/vectors, fake-device session tests, CLI tests
cargo clippy --all-targets
cargo build --release            # target/release/uclog

# Optional: decode every format in a directory of .logdata files
UCLOG_LOGDATA_CORPUS=~/.cache/uclog cargo test -p uclog-logdata --release -- --ignored corpus
```

```sh
uclog list [--json] [--ble-scan 5]
uclog term -d serial:/dev/ttyACM0 [--logs] [--raw] [--port 0] [-c "version s" --exit-after 2]
uclog decode -l build/zephyr/zephyr.elf capture.bin
uclog fw-update -d auto build/zephyr/zephyr.signed.bin
```

`-v` logs info, `-vv` adds a hex trace of every byte sent and received (`uclog::wire`).

Linux builds need `libudev-dev` and `libdbus-1-dev`. On macOS, BLE from a command-line binary
needs an embedded `Info.plist` with `NSBluetoothAlwaysUsageDescription` (release packaging,
hub plan task 10).
