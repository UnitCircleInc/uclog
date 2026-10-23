# Native workspace (Rust)

Phases 1 and 2 of [docs/plans](../docs/plans/README.md): the shared crates, the `uclog`
subcommands, and the hub. `gui` and `tui` come in later phases.

| Crate | Contents |
|---|---|
| `uclog-proto` | COBS, frame scanner, 2-byte peek, mux, CBOR helpers, CRC-32C, FW-update messages, semihost codec, link chunking. No I/O (WASM-friendly) |
| `uclog-logdata` | `.logdata` / ELF `.logdata_cbor` loader, log decoding, C-printf formatter |
| `uclog-semihost` | sandboxed port-63 handler, newlib errno mapping, host clock sources (QPC / `CLOCK_MONOTONIC`) |
| `uclog-link` | Serial, HID, BLE, UDP (Don't Fragment), TCP and WebSocket links; the simulated `loopback:` device; device URIs; USB/HID/BLE/mDNS discovery |
| `uclog-session` | pulses and wake gap, liveness, dispatch, logdata resolver, semihost bridge, FW update (59 → 3) |
| `uclog-hub` | the hub ([plan 01](../docs/plans/01-hub-server.md)): backlog ring, device task, raw/WebSocket/TLS on one port, UDP clients, admission, port-63 routing, mDNS, local registry. Library plus the small `uclog-hub` executable |
| `uclog` | the executable: `list`, `decode`, `term`, `fw-update`, `hub` |

```sh
cargo test                       # unit tests, spec/vectors, fake-device session tests, hub tests, CLI tests
cargo clippy --all-targets
cargo build --release            # target/release/uclog and target/release/uclog-hub

# Optional: decode every format in a directory of .logdata files
UCLOG_LOGDATA_CORPUS=~/.cache/uclog cargo test -p uclog-logdata --release -- --ignored corpus
```

```sh
uclog list [--json] [--ble-scan 5]                 # devices, local hubs (registry) and remote hubs (mDNS)
uclog term -d serial:/dev/ttyACM0 [--logs] [--raw] [--port 0] [-c "version s" --exit-after 2]
uclog decode -l build/zephyr/zephyr.elf capture.bin
uclog fw-update -d auto build/zephyr/zephyr.signed.bin
```

`-v` logs info, `-vv` adds a hex trace of every byte sent and received (`uclog::wire`).

## Hub

```sh
uclog hub -d auto                                  # or: uclog-hub -d auto
uclog term -d tcp://localhost:9000                 # raw TCP client
uclog term -d ws://localhost:9000                  # WebSocket client (same port)
uclog hub -d serial:/dev/ttyACM0 --udp --allow-from 192.168.1.0/24 --tls-cert c.pem --tls-key k.pem
uclog hub -d auto --syscalls --files-dir ./files   # the hub answers all device syscalls itself
uclog hub -d loopback:rate=10 --port 0 --json-ready --exit-on-stdin-eof   # simulated device, for tests
```

- **One client at a time.** A second client is turned away: raw TCP is closed without data, and
  WebSocket gets close code 4001. Both show as "hub busy" in `uclog`.
- **Port 9000**, or the next free port up to 9019 when `--port` is not given. It serves raw TCP,
  WebSocket (`/` or `/uclog`) and, with `--tls-cert/--tls-key`, TLS and `wss://`.
- **Admission:**
  - Clients on this machine are accepted. The exception is a browser page whose `Origin` is not on
    `--allow-origin`.
  - Others are accepted with `--allow-all`, `--allow-from CIDR`, or the saved allow-list.
  - Otherwise the operator is asked on the console: yes, no, or always (saved to `hub.allow` in the
    config directory). Without a console the client is rejected, and a log line says how to allow
    it.
- **Backlog:**
  - Device frames are kept while no client is connected (`--backlog`, default 1 MiB).
  - A new client gets the device info that applies to its first frame, then the backlog, then live
    data.
  - A client that falls behind skips frames (counted), or is disconnected with
    `--slow-client disconnect`.
- **Syscalls:**
  - The hub answers the time ops itself, always.
  - Other syscalls go to the connected client. With no client they get `ENOSYS`; with `--syscalls`
    the hub answers them all.
  - If a client leaves while the device waits for a reply, the device gets `EIO`.
- **Discovery:**
  - mDNS: `_uclog._tcp`, plus `_uclog._udp` with `--udp`. The TXT record carries `busy`.
  - A registry file per hub in the runtime directory, so `uclog list` finds local hubs without mDNS.
- **Testing:** `-d loopback:` is a simulated device. Decode it with `-l spec/vectors/sample.logdata`.
  It answers `help` on port 61.

## Platforms

Linux builds need `libudev-dev` and `libdbus-1-dev`. `packaging/linux/70-uclog.rules` gives users
access to Zephyr USB devices.

On macOS, `build.rs` embeds `packaging/macos/Info.plist` into both executables. Its
`NSBluetoothAlwaysUsageDescription` lets CoreBluetooth work from the command line.

Release builds: `.github/workflows/release.yml` runs on a `v*` tag and builds Linux, macOS and
Windows (x64 and arm64) archives with checksums. Code signing, notarisation and armv7 are not set
up yet.
