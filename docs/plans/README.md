# ucLog client re-architecture — overview

This directory holds six separate plans plus this shared overview:

| Plan | File | Code family |
|---|---|---|
| Hub (Serial/HID/BLE/UDP ↔ TCP/WebSocket/UDP) | [01-hub-server.md](01-hub-server.md) | Native (Rust) |
| Web version (Chrome) | [02-web.md](02-web.md) | Web (TypeScript) |
| VS Code extension | [03-vscode.md](03-vscode.md) | Web (TypeScript) + bundled native hub |
| Native graphical client | [04-native-gui.md](04-native-gui.md) | Native (Rust) |
| Native TUI client | [05-tui.md](05-tui.md) | Native (Rust) |
| Python client library for scripts | [06-python-client.md](06-python-client.md) | Python (uses the native hub) |

Read this overview first. It records the decisions made so far (§1), the wire protocol (§3),
the shared architecture (§4), cross-cutting requirements (§5), and the **open questions that are
still unresolved** (§7).

---

## 1. Decisions (answers to review rounds 1–4 and the firmware review)

| # | Topic | Decision |
|---|---|---|
| 1 | USB filter | Default serial filter = STLINK + FTDI **+ Zephyr default CDC-ACM (`2FE3:*`)**. HID stays VID `2FE3`, interface 1 |
| 2 | Hub framing awareness | Hub COBS-delimits frames and peeks at most **2 encoded bytes** to get the port (§3.2). Port 63 frames are fully decoded/encoded (§3.5). Nothing else is decoded |
| 3 | Multiple clients | **v1: exactly one client at a time** (TCP, WebSocket or UDP). Connecting implies the port-63 syscall opt-in. Revisit multi-client if a real use case appears |
| 4 | `--syscalls` | The hub fully handles port 63. stdin `read` → EOF, stdout/stderr `write` → discarded. **Without `--syscalls` and with no client connected**, the hub answers only the time calls; every other call returns an error. The hub **always** answers the time calls itself (decision 31) |
| 5 | Device/link liveness | No status port. Liveness comes from **pulses** (lowest common denominator; required for UART, where unplugging the cable or resetting the device is invisible to the host). The device powers down UART RX after 2 s without a pulse and wakes on an RX edge (§3.1) |
| 6 | No client connected | The hub **keeps reading the device**: it caches device info (port 62) and keeps a **configurable circular backlog** of raw frames, without decoding them (oldest dropped first) |
| 7, 8 | Echo to other clients; broadcast request/response | N/A: single client |
| 9 | TCP and WebSocket port | **Same port** (default 9000), using protocol sniffing (§4 of the hub plan). An optional separate `--ws-port` is available |
| 10 | Web → hub reachability | Confirmed: the web version uses `ws://localhost:*`, or `wss://` for remote hubs |
| 11 | objdump/DWARF fallback | Confirmed: dropped. Supported formats are `.logdata` and ELF `.logdata_cbor` |
| 12 | UDP | Kept as a transport everywhere it applies. Hub: UDP **device side** (network device → hub) and UDP **client side** (hub serves one UDP client). Not in browsers |
| 13 | Native language | **Rust**: GUI, TUI and hub ship as one small, self-contained executable from GitHub Releases. No Python/Node runtime. **No DearPyGui reuse** |
| 14 | Incoming connections | Localhost connections are accepted. **Non-localhost connections need user confirmation by default**. `--allow-all` (and `--allow-from CIDR`) override |
| 15 | Semihost sandbox | Fully sandboxed in every implementation |
| 16 | Timestamps | Not planned (client receive time) |
| 17 | mDNS instance name | `"{hostname}-{interface}"`, e.g. `labpc-ttyACM0`, `labpc-COM3`, `labpc-hid-<serial>`, `labpc-ble-<addr>`, `labpc-udp-<ip>` |
| 18 | BLE chunk size | Automatic. The device copes with any size. Write the largest chunk the platform allows; no user setting |
| 19 | VS Code + hub | Spawn the hub automatically. **Bundle all OS/arch hub binaries in one VSIX** and choose one at run time |
| 20 | FW update | **New port 59**: plain CBOR, no inner COBS and no CRC (decision 22). Clients try port 59 first and fall back to legacy port 3 (nested COBS) if there is no response. Port 3 support is removed later |
| 21 | Semihost time ops | ARM-semihosting opcodes: **`0x10` SYS_CLOCK**, **`0x11` SYS_TIME**, **`0x30` SYS_ELAPSED**, **`0x31` SYS_TICKFREQ** (§3.5) |
| 22 | Port 59 integrity | **No CRC32C.** Port 59 payload = one CBOR map. COBS gives framing. Image integrity comes from the **RFC 8032 (Ed25519) signature in the image header**, so a corrupted image can never be accepted; a corrupted transfer fails at `fw-schedule` |
| 23 | Hub backlog | Confirmed: every non-pulse device frame except port 63, in order, default 1 MiB, replayed after the port-62 frame in effect at the oldest entry |
| 24 | Python scripting | **New small Python client library** ([Plan 06](06-python-client.md)): start a hub if none is running, choose a hub when several run (user supplies the address), send/receive, load CBOR logdata and decode logs |
| 25 | errno | The firmware uses **newlib errno numbers** (for now). Every host maps its errors to them |
| 26 | Raw-script clients | Hub flag **`--client-syscalls off`** accepted: the hub's default handler keeps answering port 63 while such a client is connected |
| 27 | Current Python code | **No hot-fixes.** It is frozen and replaced by the native apps and the Plan 06 library |
| 28 | Rust vs Go | **Rust, confirmed** (§4 "Why Rust"). Keep the option open to move Rust code to **WebAssembly** for the web and VS Code versions later (§4.1) |
| 29 | Time-op semantics | SYS_CLOCK = u32 centiseconds, wraps, only deltas matter. SYS_TIME = 64-bit. SYS_ELAPSED = 64-bit raw host monotonic counter (QPC / `CLOCK_MONOTONIC`). SYS_TICKFREQ = that counter's frequency. Time ops do not set errno. TIMEOFDAY = same clock as SYS_TIME. SYS_TICKFREQ is **64-bit**, to match `QueryPerformanceFrequency` |
| 31 | Who answers time calls at a hub | **The hub always answers the time calls itself** (SYS_CLOCK, SYS_TIME, SYS_ELAPSED, SYS_TICKFREQ, TIMEOFDAY), whether or not a client is connected. They are never forwarded. The device sees one continuous clock for the hub's lifetime |
| 32 | Port 63 op set | **The ucLog op set implemented by the legacy `uclog/protocol/semihost.py`** (ops 0–13, incl. FSTAT, MKDIR and TIMEOFDAY), plus the time ops `0x10 0x11 0x30 0x31` (decision 21). This is *not* ARM semihosting; moving to [ARM semihosting](https://github.com/ARM-software/abi-aa/blob/main/semihosting/semihosting.rst) is a later, separate transition |
| 30 | Python-started hubs | A hub started by the Python library is **stopped when the script exits**. Hubs that were already running are left alone. FW update from Python is out of scope for v1 (`uclog fw-update`), possibly added later |

---

## 2. Current state (what the Python code does today) and issue disposition

- **Transports** (`uclog/transport/`): Serial (pyserial), HID (hidapi), BLE (bleak), TCP/UDP
  client. Each runs on its own thread. Serial, HID and TCP send their own 0.5 s `\x00` pulse.
- **Protocol pipeline** (`uclog/protocol/`): `CobsDecode → MuxDecode → {port handlers | LogDecode}`.
- **Servers**: `LogServer` (`uclog-cli.py -s`) opens one TCP server **per port** (base+1+N) plus a
  *decoded* CBOR log stream on the base port. `LogClientServer` (GUI) opens per-port servers for
  ports it does not handle, for scripts (`logcmd.py`, `StreamClient`, FW update).
- **FW update** (`gui/UnitUpdate.py`) uses `StreamClient` to loop back through the GUI's own port-3 server.
- **Decoding** (`uclog/logdata.py`): `.logdata` CBOR, ELF `.logdata_cbor`, or an objdump/DWARF fallback.
- **App-hash matching** (`uclog-gui.py:app_hash_handler`): manual → `~/.cache/uclog` →
  `wavespar.com/logdata` → the file next to the last FW image.

| # | Issue found | Disposition |
|---|---|---|
| 1 | `LogServer` mDNS reads `target_info.serial_number` from a dict, so `uclog-cli.py -s` fails | **Fix** in the new hub (hub plan §7). No hot-fix in the current Python (decision 27) |
| 2 | GUI "Servers" menu treats `LogServer`'s decoded base port as a raw stream | Removed by design: the hub carries only the raw wire protocol |
| 3 | macOS objdump fallback runs a Linux binary | Removed (decision 11) |
| 4 | `sh_open` does not keep paths inside the sandbox | Fixed by the full sandbox (decision 15) |
| 5 | Semihost file path is a module global | Fixed: per-instance state |
| 6 | `fid & 0x7ffffff` (27-bit mask) | **Fix**: `0x7fffffff`, bit 31 = "no response required" |
| 7 | Windows HID polls with `get_input_report` | **Fix where possible**: use interrupt IN reads (hidapi `read_timeout`, overlapped `ReadFile`) on every OS. Keep polling only as a fallback if the firmware turns out to answer GET_REPORT only (verify on hardware) |
| 8 | Decode errors silently dropped | **Fix**: every failure becomes a formatted error record in the log view (§5.6) |
| 9 | *(new)* Semihost replies carry the **host** `errno`, which differs from newlib's numbering on macOS/Windows | Map host errors to newlib errno values (decision 25) |
| 10 | *(new)* An unknown semihost op gets no reply, so the device waits forever | Reply with the generic error reply `<i32 -1, i32 ENOSYS>` (§3.5) |

---

## 3. Wire protocol (`spec/protocol.md` + `spec/vectors/`)

> **Phase 0:** the normative text is now [`spec/protocol.md`](../../spec/protocol.md), with vectors
> in [`spec/vectors/`](../../spec/vectors/README.md). This section is the planning summary. Where
> the two differ, `spec/protocol.md` wins. Phase 0 clarifications:
> - empty runs between `00` delimiters are ignored, and liveness is "any byte received";
> - bytes before the first `00` are dropped;
> - `%s` arguments use a defined escaping instead of Python `repr()`;
> - the FSTAT error reply is zero padded to 48 bytes;
> - a malformed request gets `EINVAL`;
> - the wake gap after more than 2 s idle is ≥ 50 ms (USB-UART converters buffer);
> - port 63 keeps the ucLog op set from the legacy `uclog/protocol/semihost.py` (decision 32).

The protocol is unchanged except for **new port 59** and **four new semihost ops**. TCP, WebSocket and UDP links between hub and
client carry exactly the bytes a device sends over serial.

### 3.1 Framing, pulses, wake-up

- Each frame is COBS-encoded and delimited by `0x00`. Encoders emit `00 <cobs> 00`, and the
  **leading `00` is always sent**. It also serves as the UART wake byte (below).
- **Pulse** = an empty frame: a lone `00`, or `00 01 00` (COBS of an empty payload).
  - Host → device: a pulse whenever nothing was sent for 0.5 s. The hub pulses the device
    **continuously** (decision 6), whether or not a client is connected.
  - Device → host: the device sends idle pulses too. These are forwarded to the client.
  - Client → hub: clients pulse the same way. The hub uses them for client liveness and never
    forwards them to the device.
- **Liveness**: no bytes for more than **2 s** means the peer is gone, on every link. The device uses
  the same rule. After 2 s without a pulse it powers down UART RX and only watches for edges. The
  first edge re-enables RX, so **the first byte(s) after a long idle are sacrificial**. Senders must
  pulse at least every 0.5 s while connected. After any idle of more than 2 s (for example, a fresh
  connection), a sender must send a pulse and wait ≥ 50 ms before the first real frame (USB-UART
  converters buffer before the edge reaches the target).
- Maximum encoded frame size: 4096 bytes (`--max-frame`). Longer input is discarded up to the next
  `00` and counted as a framing error.

### 3.2 Frame header and the 2-byte peek

The first decoded byte `b0`: `type = b0 & 3`; when `type == 3` (port frame), `port = b0 >> 2`.

The hub finds `b0` without decoding the frame. With encoded frame `e`:

- `len(e) == 0`, or `e == [01]` → pulse.
- `e[0] == 01` → `b0 = 0x00` (a type-0 log frame, not a port frame).
- otherwise → `b0 = e[1]`.

| type | Meaning | Layout |
|---|---|---|
| 0 | printf-style log | `u32le addr` (low 2 bits = type) + packed args; target digit = `(addr>>20)&0xF` |
| 1 | memory dump log | `u32le addr` + `u32 ptr` + bytes |
| 2 | reserved | shown as an undecodable-frame error record |
| 3 | port frame | `port = b0>>2` (0‥63), payload = rest |

### 3.3 Ports

| Port | Use | Payload |
|---|---|---|
| 0 | Commands | CBOR map, or text + `\r` |
| 1 | "Prism" stream | CBOR, logged only |
| 3 | FW update (**legacy**, to be removed) | nested `00 cobs(cbor ‖ crc32c_le) 00` |
| **59** | **FW update (new)** | one CBOR map, **no inner COBS, no CRC**. Same commands as port 3 (§3.4) |
| 60 | Plotting | CBOR: `plot`/`heatmap`/`spectrogram` create, stream (`x`, `yAS`), `clear_plot` |
| 61 | Terminal | UTF-8 + VT100; host→device keystrokes/lines |
| 62 | Device info | CBOR map incl. `app_hash` (64 bytes), `image_type`, `board`, … |
| 63 | Syscalls (semihost) | `u8 op` + args → reply (§3.5) |

### 3.4 FW update (ports 59 / 3)

Commands are CBOR maps: `set-time{timestamp}` → `fw-start` (reply has `block-size`; may take up
to 60 s) → `fw-block{data}` × N → `fw-schedule{timestamp}`. Each reply has `rsp == cmd`, and
`event` messages are ignored.

**Port selection:** send `set-time` on port 59. If a valid reply arrives within 1 s, use 59 for the
whole update. Otherwise send `set-time` on port 3 (nested format). If that also fails → error
"device does not support FW update". The chosen port is cached per device serial for the session.

### 3.5 Port 63 — semihost

- Request `u8 op ‖ args`. Existing ops: 0 SYSTEM, 1 RENAME, 2 ISATTY, 3 UNLINK, 4 LINK, 5 STAT,
  6 FSTAT, 7 CLOSE, 8 OPEN, 9 WRITE, 10 LSEEK, 11 READ, 12 MKDIR, 13 TIMEOFDAY.
- **New time ops** (decisions 21, 29). The opcodes match ARM semihosting and do not collide with
  0–13. Requests carry no arguments. These calls **always succeed and never set errno**, so the
  reply is just the value, little-endian, with no `<r, errno>` prefix:

  | Op | Name | Reply | Host value |
  |---|---|---|---|
  | `0x10` | SYS_CLOCK | `u32` centiseconds | the host's monotonic clock / 10 ms. It wraps, which is expected: the start point is irrelevant because only deltas are used. A low-resolution SYS_ELAPSED |
  | `0x11` | SYS_TIME | `i64` seconds | wall-clock seconds since 1970-01-01 UTC (`time_t` is 64-bit on current arm-none-eabi gcc) |
  | `0x30` | SYS_ELAPSED | `u64` ticks (same bytes as `u32 lo, u32 hi`) | the raw host monotonic counter: `QueryPerformanceCounter` (Windows), `clock_gettime(CLOCK_MONOTONIC)` in ns (Linux/macOS/POSIX) |
  | `0x31` | SYS_TICKFREQ | `u64` ticks/s | matches SYS_ELAPSED: `QueryPerformanceFrequency` (typically 10 000 000) on Windows; 1 000 000 000 for `CLOCK_MONOTONIC` ns |

  - Per-implementation sources: Rust uses the OS calls above directly (not `Instant`, which hides
    the raw counter). Python uses `time.perf_counter_ns()`, ticks/s 10⁹. Browsers/VS Code
    webviews use `performance.timeOrigin + performance.now()` in µs, ticks/s 10⁶. Their real
    resolution is coarser (≈ 5–100 µs), which is acceptable.
  - **At a hub, the hub always answers the time calls** (decision 31), so the counter and its
    frequency never change while the device stays on that hub. A client connected directly to the
    device (GUI/TUI/web) answers them itself. The base and frequency then change only when the
    device is moved to a different host or link.
  - `TIMEOFDAY` (13) is the same clock as SYS_TIME and keeps its existing reply
    `<u64 tv_sec, u32 tv_usec>`. Every answerer, including the hub's default handler, supports both.
- `WRITE` fid bit 31 = no reply wanted; fid = `x & 0x7fffffff` (issue 6).
- **Generic error reply** (file ops and unsupported ops, issue 10): `<i32 -1, i32 errno>`. Every
  file op already starts its reply with `<i32 r, i32 errno>`. The time ops have no error form.
- Errors use **newlib errno numbering** (decision 25, issue 9). Unsupported op → `ENOSYS` (88 in
  newlib).
- **Policy, the same in every implementation:**

| Situation | Who answers port 63 | What is supported |
|---|---|---|
| Hub `--syscalls` | hub | everything, sandboxed to `--files-dir`; stdin EOF, stdout/stderr discarded |
| Hub, any mode, **time calls** | hub, always (decision 31) | SYS_CLOCK, SYS_TIME, SYS_ELAPSED, SYS_TICKFREQ, TIMEOFDAY. Never forwarded |
| Hub, client connected (no `--syscalls`) | client (connection = opt-in) | all other ops: the client's implementation (file calls need a sandbox dir; otherwise `ENOSYS`/`EACCES`) |
| Hub, no client (no `--syscalls`) | hub | time calls only; others → `<i32 -1, i32 ENOSYS>` |
| Client disconnects with a request outstanding | hub | hub replies with error (`EIO`) so the device does not hang; hub tracks the outstanding request |
| GUI/TUI/web connected directly to device | that client | its implementation, sandboxed |

### 3.6 Link specifics

- **Serial**: 1 000 000 baud 8N1. STLINK workaround: split writes when `len % 8 == 0` (re-test on
  current STLINK firmware).
- **HID**: in report `0xA1`, out report `0xA2`, 63-byte payload chunks (zero padding is harmless).
  Interrupt reads on all OSes (issue 7).
- **BLE**: service and characteristic UUID `8683b8f3-0000-1000-8000-00805f9b34fb`. Writes use the
  largest chunk the platform allows (decision 18); notifications carry device → host data.
- **UDP**: each datagram carries one or more whole frames (`00`-delimited); no reassembly across
  datagrams.

`spec/vectors/` contains COBS cases (from `uccobs.py`), header-peek cases, mux frames, log frames with
a small `.logdata` and expected text, port-59 and port-3 FW update frames, semihost request/reply
pairs, and HID chunking. The Rust and TypeScript implementations both run them in CI.

---

## 4. Target architecture

```
 ┌──────────────────── Native: one Rust workspace → `uclog` executable ─────────────────────┐
 │ uclog-proto     COBS, scanner/peek, mux, ports, FW-update msgs, semihost codec          │
 │ uclog-logdata   .logdata / ELF .logdata_cbor loader, printf formatter                   │
 │ uclog-semihost  sandboxed FS + time calls, newlib errno mapping                         │
 │ uclog-link      Serial, HID, BLE, UDP, TCP, WS links; discovery (USB/HID/BLE/mDNS)      │
 │ uclog-session   pulse+liveness, dispatch, services: decoder, resolver, fw-update,      │
 │                 terminal, commands, plot model, syscalls                                │
 │ uclog-hub       hub engine (Plan 01)                                                    │
 │ uclog-tui       ratatui front end (Plan 05)                                             │
 │ uclog-gui       egui front end (Plan 04)                                                │
 │ uclog (bin)     subcommands: gui | tui | hub | term | fw-update | decode | logdata | list│
 └───────────────────────────────────────────────────────────────────────────────────────────┘
 ┌──────────────────── Web: TypeScript pnpm workspace ──────────────────────────────────────┐
 │ @uclog/core, @uclog/transports-web, @uclog/ui (Lit), @uclog/element (<uc-log>)          │
 │ apps/demo, apps/vscode (bundles native `uclog-hub` binaries)                            │
 └───────────────────────────────────────────────────────────────────────────────────────────┘
 ┌──────────────────── Python: `uclog` package (Plan 06) ───────────────────────────────────┐
 │ hub discovery/spawn, Connection (pulse, ports, time syscalls), logdata loader + decoder │
 │ platform wheels bundle the native `uclog-hub` binary                                    │
 └───────────────────────────────────────────────────────────────────────────────────────────┘
```

### Why Rust over Go (decisions 13, 28)

Both produce a single self-contained executable, and both are fine for the hub and the TUI alone.
**The GUI decides it.** Go's usual advantages are mostly lost for this project once a GUI, HID and
BLE are involved.

| Concern | Rust | Go |
|---|---|---|
| **GUI with dense plots** (3 y-axes, cursors, heatmap/spectrogram at high rates, virtual tables, docking) | `egui` + `egui_plot` + `egui_extras` + `egui_dock`: mature immediate-mode stack, already used for real-time tools. Multi-axis needs a shim (Plan 04 risk) | Fyne / Gio have no comparable plotting widget. Plots, multi-axis, heatmaps and a fast virtual table would all be custom-drawn. The largest single piece of work would grow |
| **Cross-compiling** | Needs per-OS CI runners (C code in `hidapi`, macOS signing) | Go's easy cross-compile needs `CGO_ENABLED=0`. HID (`go-hid`/`karalabe/hid`), the GUI (Fyne/GLFW; Gio on Linux) and BLE on macOS all need **cgo**, so Go also ends up on per-OS runners. No advantage left |
| **Libraries for the rest** | `tokio`, `serialport`, `hidapi`, `btleplug`, `mdns-sd`, `tokio-tungstenite` + `rustls`, `ciborium`, `object` (ELF), `reqwest`, `keyring`, `rfd`, `ratatui` | Comparable: `go.bug.st/serial`, `tinygo-org/bluetooth`, `hashicorp/mdns`, `gorilla/websocket`, `fxamacker/cbor`, `debug/elf` (stdlib), `bubbletea`. Roughly a tie |
| **Executable size** | `uclog-hub` ≈ 4–6 MB; full `uclog` with egui/glow ≈ 10–15 MB (estimates) | Hub ≈ 8–12 MB; with Fyne ≈ 25–35 MB (estimates) |
| **Latency / memory** | No GC; the bounded backlog ring and per-frame buffers are explicit | GC is fine at these rates. Not decisive |
| **Sharing with the web version** | The core (COBS, peek, logdata decoding) could later compile to **WASM** (≈ 100–300 KB) and replace the TypeScript port, so there is one decoder. Optional; Plan 02 does not depend on it | Go WASM carries a ≈ 2 MB runtime; TinyGo is smaller but limited. Not practical for an embeddable component |
| **Firmware team fit** | Closer to the C mindset (explicit memory, no runtime). Zephyr is adding Rust support | Different idioms from embedded C |
| **Learning curve / build speed** | Steeper; slower compiles | Easier to learn and review; fast compiles. **Go's main advantage** |

**Recommendation:** Rust. Go would be a reasonable choice if the GUI were dropped, or if the team
is fluent in Go and not in Rust. In that case, plan for a custom plotting/table layer in Plan 04,
which is about +L in size. **Confirmed: Rust** (decision 28).

### 4.1 Keeping the WebAssembly option open (decision 28)

The web and VS Code versions start with the TypeScript core (Plan 02). To allow moving Rust code
to WASM later without a rewrite:

- `uclog-proto`, `uclog-logdata` and the pure parts of `uclog-semihost` (codec, time-op encoding,
  errno map) are **`no_std` + `alloc`-friendly, with no I/O, threads or tokio**. Time sources and
  file systems are passed in as traits. CI builds them for `wasm32-unknown-unknown`.
- A thin `uclog-wasm` crate (`wasm-bindgen`) can later expose `FrameScanner`, `peek`,
  `LogData.load/decode` and the semihost codec. Plan 02 keeps these behind the same TypeScript
  interfaces (`@uclog/core`), so the TS implementation can be swapped per module.
- The shared `spec/vectors/` are the acceptance test for each swap: the WASM module must pass the
  same vectors as the TS one.
- Most likely first candidate: the log decoder, so that formatting is identical in native and web
  and the code is maintained once. Budget: about 150–300 KB of WASM, lazy-loaded in the decode
  worker.

**Build variants** from the same workspace (Cargo features):

- `uclog`: gui + tui + hub + tools. Target ≤ 15 MB stripped with `glow` rendering.
- `uclog-hub`: hub + tools only, no GUI libraries. About 4–6 MB. For VS Code bundling, Raspberry Pi
  and servers.

Linux runtime dependencies: glibc, `libudev` (HID) and `libdbus-1` (BLE) are present on mainstream
distros. GUI display libraries (X11/Wayland) are loaded at run time, so `uclog hub` and `uclog tui`
run on headless machines.

### Connection model

- **GUI and TUI** open the device **directly** (Session over a Link) or connect to a hub
  (`tcp://`/`udp://`). With single-client hubs there is no in-process hub. To share a device or use
  scripts alongside, run `uclog hub` and connect everything to it.
- **Web** uses Web Serial/HID/Bluetooth directly, or a hub over `ws(s)://`.
- **VS Code** always goes through a hub. It spawns a private local hub for USB/BLE, or connects to a
  remote one.
- **Python scripts** ([Plan 06](06-python-client.md)) always go through a hub. The library finds
  one, or starts one if none is running. ⚠ With a single-client hub, a script and a GUI cannot be
  connected to the same device at the same time; the second one gets "hub busy".

### Repository layout (`uclog-clientv2`, a new repository)

```
spec/          protocol.md, vectors/, reference/ (stdlib-only Python reference + vector generator),
               firmware-changes.md
docs/plans/    these plans
native/        Cargo workspace (crates above), packaging/, udev/
web/           pnpm workspace (packages/, apps/demo, apps/vscode)
python/        new `uclog` client library for scripts (Plan 06)
```

The legacy Python client stays in its own repository (`uclog-client`), frozen (decision 27), until
the native GUI/TUI/hub and the Plan 06 library reach parity.

---

## 5. Cross-cutting requirements

### 5.1 Device discovery and selection

- **Serial default filter** (override with `--usb-filter VID:PID`, repeatable, or config):
  - STLINK `0483:374B`, `0483:374E`, `0483:374F`, `0483:3753`, `0483:3754`
  - FTDI `0403:6001`, `0403:6010`, `0403:6011`, `0403:6014`, `0403:6015`
  - Zephyr CDC-ACM `2FE3:*`
- **HID**: VID `0x2FE3`, interface 1. **BLE**: scan filtered on the service UUID.
- **Hubs**: mDNS `_uclog._tcp` / `_uclog._udp` (native and VS Code), or a manual address.
- Every client shows **one list** (kind, name, serial, description) for the user to pick from.
  Auto-connect only when exactly one candidate matches, or when `--device` is given.

### 5.2 Connection policy for hubs (decisions 3, 14)

- One client at a time. Second connection attempt:
  - TCP: closed immediately.
  - WebSocket: closed with code `4001 busy`.
  - UDP: ignored.
- Clients show "hub busy" when the connection is closed before any data arrives.
- Non-localhost connections (and browser WebSocket connections with an Origin not on the allowlist)
  are **held pending until the user confirms**. Clients show "waiting for hub approval", with a 60 s
  timeout.

### 5.3 Logdata service (`fw.unitcircle.ca`)

- `GET https://fw.unitcircle.ca/logdata/{domain}/{app_hash_hex128}`, with
  `Authorization: Bearer <token>` if a token is configured. The response is a 302 to a 5-minute link,
  which is followed.
- Settings: `logdata.domain` (required for remote lookup), `logdata.token` (optional),
  `logdata.url` (default `https://fw.unitcircle.ca`). Env: `UCLOG_LOGDATA_DOMAIN`,
  `UCLOG_LOGDATA_TOKEN`. Tokens are stored in the OS keyring (native), SecretStorage (VS Code), or
  per session (web, unless the user chooses to remember it).
- Handle `401/403/404/429 (Retry-After)` and RFC 9457 problem+json. Show `X-Request-Id`.
- Upload (`uclog logdata upload`): `PUT` with `Accept: application/json`. `200` = present,
  `201` = created, `409` = conflict.
- Resolver order: manual → local cache (`<cache>/<hash>.logdata`) → remote → FW-image sibling or
  workspace match (SHA-512 of a `*.signed.bin` after its 512-byte header equals `app_hash`).
- MCP (`https://fw.unitcircle.ca/mcp`): `list_logdata`, `get_logdata_meta`,
  `get_download_link`, `create_upload_url`. Documented for developer and AI workflows; clients and CI use REST.

### 5.4 Logdata formats

`.logdata` CBOR, or ELF with `.logdata_cbor`. No objdump/DWARF. Exact Python `%`-formatting
parity is **not** required. Use C-printf semantics with reasonable formatting of
flags/width/precision.

### 5.5 FW update

Every client offers a file picker for the `.bin`: `rfd` native dialog (GUI), in-TUI file browser,
`<input type=file>` (web), `showOpenDialog` (VS Code). Build-dir and workspace `zephyr.signed.bin`
files are offered as suggestions. Port 59 is tried first, then port 3 (§3.4). After success, the
image path seeds the logdata resolver.

### 5.6 Error records (issue 8)

Every decode or framing problem produces a record shown in the log view with level `ERROR` and
source `<uclog>`. A per-type counter is shown in the status bar. Formats:

| Case | Text |
|---|---|
| COBS invalid | `framing: invalid COBS (n bytes) near <hex…>` |
| Frame too long | `framing: frame exceeded 4096 bytes, discarded n bytes` |
| No logdata for target | `decode: no logdata for target T (addr 0x…) [<hex payload>]` |
| Unknown format address | `decode: unknown format addr 0x… in <logdata file> [<hex>]` |
| Argument underrun/overrun | `decode: <file>:<line> "<fmt>" — missing arg k (int32)` / `extra n bytes: <hex>` |
| CBOR decode error on port N | `port N: invalid CBOR (<error>) [<hex…>]` |
| Unknown port | `port N: no handler (n bytes)` (shown once per port, then counted) |

### 5.7 CLI (single `uclog` executable, no backward compatibility)

| Subcommand | Replaces |
|---|---|
| `uclog gui` | `uclog-gui.py` |
| `uclog tui` | `uclog-cli.py` display / `-c` |
| `uclog hub` | `uclog-cli.py -s` (`LogServer`) |
| `uclog term` | `logcmd.py` (now works on Windows) |
| `uclog fw-update FILE` | GUI-only today |
| `uclog decode -l F FILE` | `logfile.py` |
| `uclog logdata upload --bin B --logdata L` / `uclog logdata dump F` | `cachelogdata.py` / `python -m uclog.logdata` |
| `uclog list [--json]` | `print_targets_found` |

| Common flag | Meaning | Replaces |
|---|---|---|
| `-d, --device URI` | `serial:/dev/ttyACM0` (`serial:COM3`), `hid:<path or serial>`, `ble:<addr or name>`, `udp://host:port`, `tcp://host[:9000]`, `ws(s)://host[:9000]`, `auto` (default) | `--target`, `--host`, `-c` |
| `--baud N` | default 1000000 | `--baudrate` |
| `--usb-filter VID:PID` | repeatable; replaces defaults | — |
| `-l, --logdata FILE` | repeatable manual logdata | `-e` |
| `--cache-dir DIR` | logdata cache | `--cache` |
| `--logdata-domain D` / `--logdata-token T` / `--logdata-url U` | logdata service | `--logserver`, `LOGDATA_TOKEN_GET` |
| `--files-dir DIR` | semihost sandbox root | `file_path` |
| `--config FILE` | settings (platform config dir by default) | `~/ucLogger.json` |
| `-v` / `-vv` | log level / wire trace | `-v` |

Hub flags are in the hub plan.

---

## 6. Phasing

| Phase | Work | Unblocks |
|---|---|---|
| 0 | ✅ Questions closed (§7). ✅ `spec/protocol.md`, reference, vectors and CI. ✅ Firmware requirements agreed (F1–F5, C1–C10 in [`spec/firmware-changes.md`](../../spec/firmware-changes.md)). ⏳ Firmware implementation of F1–F5 (outside this repository) | everything |
| 1 | ✅ Rust crates: proto, logdata, semihost, link, session (+ `uclog list/decode/term/fw-update`) in [`native/`](../../native/README.md). ⏳ Hardware verification (serial/HID/BLE/UDP, FW update) | hub, TUI, GUI |
| 2 | Hub ([01](01-hub-server.md)) + `uclog-hub` release builds | VS Code; remote use; Python library |
| 2b | Python library ([06](06-python-client.md)), in parallel with Phase 3 | script users; retire the legacy repository |
| 3 | TUI ([05](05-tui.md)), the smallest complete front end, validates Phase 1 | GUI |
| 4 | GUI ([04](04-native-gui.md)) | retire Python |
| 5 | Web ([02](02-web.md)), parallel from Phase 0 | VS Code |
| 6 | VS Code ([03](03-vscode.md)) | — |

---

## 7. Open questions

None. All questions from rounds 1–4 are answered and recorded as decisions 1–31 (§1) and in §3.5.
New questions that come up during Phase 0 (`spec/protocol.md` and vectors) go here.
