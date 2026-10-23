# Plan 01 — `uclog hub`: device ↔ TCP / WebSocket / UDP bridge

Shared context, protocol and decisions: [README.md](README.md). Decisions are referenced as D#
(README §1).

## 1. Goal

A native Rust process, shipped in both the `uclog` and the small `uclog-hub` executables, that owns
**one device** and serves **one client at a time** (D3):

- **Device side**: Serial, HID, BLE, or UDP (a network device).
- **Client side**: **one listening port** (default `9000`, `--port`) that accepts **raw TCP and
  WebSocket** on the same port (D9). Optional TLS (`wss://` / TLS-TCP) and optional **UDP** client
  side (D12). Advertised over mDNS.
- Carries the unchanged wire protocol. **No log decoding**: logdata, timestamps and app-hash
  matching happen in clients. The hub's protocol knowledge is limited to: frame delimiting, a
  2-byte port peek (README §3.2), and full handling of **port 63** (README §3.5).
- Keeps reading the device when no client is connected: it caches device info and keeps a circular
  **backlog** (D6).

The current per-port servers and decoded base-port stream are removed (no backward compatibility).

## 2. Architecture (tokio)

```
                ┌────────────────────────── Hub ─────────────────────────────────────────┐
 Device Link ──►│ DeviceReader → FrameScanner → peek ─┬─ port 63 → Syscalls ─(reply)─┐   │
 (serial/hid/   │                                     ├─ pulse  → liveness          │   │
  ble/udp)  ◄───│ DeviceWriter ◄── pulse timer (0.5 s, always)                ◄──────┘   │
                │       ▲                             └─ other  → Backlog ring ──► ClientWriter ─► client
                │       └── client frames (pulses dropped) ◄── FrameScanner ◄── ClientReader ◄─ client
                │ Listener: TCP accept → sniff (TLS? / WS? / raw) → admission (busy? approve?)  │
                │ UDP listener (optional) · mDNS advertiser · stats                             │
                └──────────────────────────────────────────────────────────────────────────────┘
```

Crates: `uclog-proto` (scanner, peek, semihost codec), `uclog-semihost` (sandboxed handler),
`uclog-link` (device links), `tokio`, `tokio-tungstenite`, `tokio-rustls`, `mdns-sd`, `clap`,
`tracing`.

## 3. Device side

- Selection: `-d URI` (`serial:…`, `hid:…`, `ble:…`, `udp://ip:port`) or `auto` = exactly one
  match of the USB filters (README §5.1); with 0 or more than 1 match, exit with the candidate list.
- **Pulse to device every 0.5 s of TX idle, always**, whether or not a client is connected (D5/D6).
  The device keeps its UART RX powered while the hub runs. Every frame is written with its leading
  `00` (the wake byte).
- **On open**: a wake pulse, the 50 ms wake gap, then an empty port-62 frame to ask for the device
  info (F7, protocol.md §5.2). Without it, a hub restarted within 2 s gets no `app_hash`.
- **Liveness**: device bytes (including its idle pulses) mark it alive. No bytes for more than 2 s →
  state `silent` (logged; the client infers the same thing from silence). OS-level events
  (HID/BLE disconnect, serial port vanishing) → state `detached` → reopen loop: every 0.5 s for
  Serial/HID/UDP; BLE re-scans by address with backoff to 5 s.
- Writes keep frames whole. Link-specific chunking lives in the link: HID 63-byte reports, BLE
  largest platform chunk (D18), STLINK `len % 8` split.
- HID read: interrupt IN (`read_timeout`) on every OS (README issue 7), with a polled
  `get_input_report` fallback behind `--hid-poll` if hardware testing shows it is needed.
- Linux: exclusive lock (`flock`) on the serial device so two hubs cannot share it.

## 4. Client side

### 4.1 One port, three protocols (sniffing)

On TCP accept, the hub waits up to 500 ms for the first byte:

| First byte | Meaning | Action |
|---|---|---|
| `0x16` | TLS ClientHello | TLS handshake (requires `--tls-cert/--tls-key`, else close), then sniff again on the decrypted stream |
| `G` | HTTP `GET` | WebSocket upgrade on path `/` (or `/uclog`). Binary messages; each carries one or more whole frames |
| anything else (normally `00`, a client pulse) or timeout | raw protocol | buffered bytes go to the client FrameScanner |

This works because raw clients always start with `00` (README §3.1). `--ws-port N` optionally
**also** serves WebSocket on a separate port, in case a proxy or environment needs it. mDNS TXT
advertises `ws=same` or the port number.

### 4.2 UDP client side (optional, `--udp [--udp-port 9000]`)

- The client is identified by the source `ip:port` of its first datagram. Admission rules are the
  same as for TCP (busy / approval).
- Hub → client datagrams pack whole frames, up to 1452 bytes per datagram (protocol.md §2.4), with
  Don't Fragment set. Client → hub datagrams must contain whole frames. A datagram refused as too
  large (EMSGSIZE / ICMP fragmentation needed) is reported, not retried: UDP clients need a path
  MTU of at least 1500 (§4.4).
- Client liveness comes from pulses only (`--client-timeout`, default 5 s). After it, the slot is
  free.

### 4.3 Admission (D3, D14)

1. **Busy**: if a client is active, reject. TCP: close. WS: close `4001 busy`. UDP: ignore.
2. **Approval**:
   - Loopback peers are accepted.
   - A **browser WebSocket** from loopback whose `Origin` is not in `--allow-origin` (default:
     `http://localhost:*`, `http://127.0.0.1:*`, `vscode-webview://*`) is treated like a remote peer.
     This stops arbitrary web pages from silently driving the device.
   - Non-loopback peers are accepted if `--allow-all` is set, or the peer matches `--allow-from CIDR`
     or the saved allow-list. Otherwise they are **held pending** while the hub asks the operator:
     ```
     Incoming tcp connection from 192.168.1.20:53122 — accept? [y]es / [n]o / [a]lways this host (60 s)
     ```
     *Always* writes the host to `hub.allow` in the config dir. The connection is rejected on timeout,
     or when stdin is not a TTY (headless) and no allow rule matches; a log line explains how to allow
     it.
   - While a connection is pending it occupies the slot. A second attempt gets "busy".
3. On accept, client session start (§5).

### 4.4 Client liveness

The hub expects bytes, at least pulses, from the client. Silence longer than `--client-timeout`
(default 5 s, longer than the 2 s device rule to tolerate network jitter) or a socket error ends
the session. The hub replies to outstanding syscalls (§6) and frees the slot.

## 5. Data flow

### 5.1 Device → client and the backlog (D6, D23)

- **Backlog ring**: frames (encoded bytes + sequence number) bounded by `--backlog BYTES`
  (default 1 MiB). When full, the oldest frames are evicted. Contents: every non-pulse device frame
  **except port 63**, in arrival order, including port-62 frames.
- `info_at_oldest`: the port-62 frame in effect at the oldest backlog entry. It is updated as
  frames are evicted. `latest_info`: the most recent port-62 frame.
- **On client accept**: send `info_at_oldest` (if any), then the backlog from oldest, then live data.
  The client therefore sees each device reset or image change (new `app_hash`) exactly where it
  happened and switches decoders.
- **The live stream and the client queue are the same ring**: the client writer holds a cursor into
  the ring. The device reader never waits for the client. If a slow client's cursor falls behind
  the oldest entry, the hub skips to the oldest entry and counts the dropped frames per session
  (`--slow-client drop`, default). `--slow-client disconnect` ends the session instead.
- **Device idle pulses are forwarded** to the client (coalesced: at most one pending), but never
  stored in the backlog.
- `--backlog 0` disables buffering; only `latest_info` is cached and replayed.

### 5.2 Client → device

- A FrameScanner on the client stream; **client pulses are dropped**. Port-63 frames are syscall
  replies (§6). Everything else is forwarded verbatim, whole, to the DeviceWriter.
- If the device is `detached`, frames are dropped and counted. The client sees silence and shows
  "device not responding".

## 6. Port 63 — syscalls

The hub fully decodes port-63 frames: `uclog-proto` semihost codec + `uclog-semihost` handler.

| Mode | Device request handled by |
|---|---|
| `--syscalls` | hub, full handler sandboxed to `--files-dir` (default: CWD). stdin `READ(0)` → 0 bytes (EOF). stdout/stderr `WRITE(1/2)` → discarded (reply success with full length). With `-v`, a summary goes to the log |
| **time calls, every mode** | hub, always (D31): `SYS_CLOCK` (0x10), `SYS_TIME` (0x11), `SYS_ELAPSED` (0x30), `SYS_TICKFREQ` (0x31), `TIMEOFDAY` (13). Never forwarded to the client and never tracked as outstanding |
| no client | hub **default handler**: time calls as above; all other ops → `<i32 -1, i32 ENOSYS>` (README §3.5) |
| client connected (and not `--syscalls`) | all non-time ops forwarded to the client (connection = opt-in, D3). The hub tracks the **outstanding request** (op decoded; `WRITE` with fid bit 31 needs no reply). Client port-63 frames clear it |
| client connected, `--client-syscalls off` | hub default handler (D26; for raw-script clients that do not implement port 63) |
| client disconnects / times out with an outstanding request | hub sends an error reply (`EIO`) immediately |

The sandbox (D15) applies to all paths: they resolve against `--files-dir` with canonicalisation,
and `..`, absolute paths and symlinks pointing outside are rejected with `EACCES`. The handler
keeps a per-session fd table (no host fds are exposed) and maps errno to newlib values (README
issue 9).

## 7. mDNS (D17)

- Service `_uclog._tcp.local.` (plus `_uclog._udp.local.` with `--udp`). Instance
  `"{hostname}-{interface}"`, where interface is the sanitised device name: `ttyACM0`, `COM3`,
  `hid-<serial>`, `ble-<addr>`, `udp-<ip>`. It is truncated to 63 bytes, and mdns-sd adds a numeric
  suffix on conflict.
- TXT: `serial_number`, `description`, `interface`, `kind` (serial/hid/ble/udp), `proto=1`,
  `ws=same|<port>`, `tls=0|1`, `busy=0|1` (updated on connect/disconnect, so pickers can grey out
  busy hubs).
- All non-loopback interfaces. `--no-mdns` disables it. Re-announced when the device identity
  changes.

### 7.1 Local registry

mDNS may be disabled (`--no-mdns`) or blocked by a firewall, and it is slow to query from scripts.
Each running hub therefore also writes a small file `<runtime dir>/uclog/hubs/<pid>.json`:

```json
{"pid": 4242, "port": 9000, "bind": "0.0.0.0", "name": "labpc-ttyACM0", "device": "serial:/dev/ttyACM0",
 "serial_number": "0042…", "kind": "serial", "busy": false, "version": "1.0.0"}
```

- Runtime dir: `$XDG_RUNTIME_DIR/uclog` (Linux, falling back to the cache dir),
  `~/Library/Caches/ca.unitcircle.uclog/run` (macOS), `%LOCALAPPDATA%\uclog\run` (Windows).
- The file is created at listen time, its `busy` field is updated on connect/disconnect, and it is
  removed on exit. Readers ignore entries whose `pid` is no longer alive (crash cleanup).
- `uclog list --json` merges registry entries (local hubs) with mDNS results (remote hubs). The
  Python library (Plan 06) and VS Code (Plan 03) use it to find local hubs.

## 8. CLI

```
uclog hub [-d URI|auto] [--baud N] [--usb-filter VID:PID]... [--hid-poll]
          [--port 9000] [--bind 0.0.0.0] [--ws-port N] [--udp [--udp-port N]] [--no-mdns]
          [--allow-all] [--allow-from CIDR]... [--allow-origin GLOB]... [--tls-cert F --tls-key F]
          [--syscalls] [--files-dir DIR] [--client-syscalls on|off]
          [--backlog BYTES] [--slow-client drop|disconnect] [--client-timeout SECS] [--max-frame N]
          [--json-ready] [--exit-on-stdin-eof] [--config FILE] [-v|-vv]
```

- **Default port in use** (for example a second hub for another device on the same machine): if
  `--port` was not given explicitly, try 9001, 9002, … up to 9019 and log the chosen port. mDNS and
  the local registry (§7.1) advertise the actual port. An explicit `--port N` that is busy is an
  error.
- `--port 0` / `--ws-port 0` / `--udp-port 0` bind ephemeral ports. `--json-ready` prints
  `{"port":N,"ws_port":M,"udp_port":U,"name":"…","device":"…"}` on stdout once listening (used by VS Code, Plan 03).
- `--exit-on-stdin-eof`: exit when stdin reaches EOF, which happens when the parent process dies.
  Used by the Python library (Plan 06) and VS Code (Plan 03) so that their private hubs never
  outlive them.
- `uclog list [--json] [--ble-scan SECS]` lists serial/HID devices (filtered), BLE devices and mDNS
  hubs (with `busy`).

## 9. Work breakdown

| # | Task | Size |
|---|---|---|
| 1 | `uclog-proto`: COBS, FrameScanner (max-frame), 2-byte peek, semihost codec; vectors | S |
| 2 | `uclog-link` device links: serial (+STLINK split, flock), HID (interrupt reads, chunking), BLE (btleplug, max chunk), UDP | L |
| 3 | `uclog-semihost`: sandboxed FS, fd table, newlib errno map, time ops, error replies | M |
| 4 | Hub core: device task, pulse timer, liveness states, backlog ring + client cursor, `info_at_oldest` | M |
| 5 | Listener: sniffing, TLS, WS upgrade + Origin, raw TCP; UDP client side | M |
| 6 | Admission: busy, allow rules, interactive approval prompt, `hub.allow` persistence | S |
| 7 | Port 63 routing state machine (modes table, outstanding-request tracking) | S |
| 8 | mDNS advertise with TXT updates; local registry file (§7.1); default-port fallback | S |
| 9 | Stats (`-v` periodic line: frames/bytes, drops, framing errors, syscall counts) | S |
| 10 | Release: `cargo-dist` (or a custom Actions matrix) for linux x64/arm64/armv7, macOS arm64/x64, windows x64/arm64; macOS embedded Info.plist with `NSBluetoothAlwaysUsageDescription` (CLI binaries need it for CoreBluetooth), codesign + notarize; Windows signing; checksums; `70-uclog.rules` udev file | M |

## 10. Testing

- **Unit**: scanner (split, merged, oversized, garbage), peek against full decode for every vector,
  backlog eviction + `info_at_oldest`, port-63 modes table, sandbox escape attempts (`..`,
  absolute, symlink), errno mapping.
- **Integration (no hardware)**: a `LoopbackLink` fake device emitting logs, pulses, port 62 changes
  and syscalls. It is exposed as a hidden `-d loopback:[script]` device so the Python library (Plan 06)
  and VS Code tests can run against a real hub binary. Tests:
  - connect after backlog fill
  - slow client
  - second client → busy
  - remote peer → prompt (with scripted stdin)
  - browser Origin not on the allowlist → prompt
  - disconnect with an outstanding syscall → device gets `EIO`
  - same-port TCP/WS/TLS sniffing
  - UDP client.
- **Hardware**: STLINK VCP, FTDI, Zephyr CDC, HID (Windows interrupt-read check, issue 7), BLE, UDP
  device. Unplug/replug and device reset during a session. Device UART power-down/wake (no pulses
  for more than 2 s, then traffic).
- **Soak**: 1 h at maximum log rate with an intermittently stalled client; memory stays bounded by
  `--backlog`.

## 11. Risks

- BLE platform differences (pairing prompts, reconnect) through `btleplug`. Max write length is not
  exposed uniformly, so small per-OS shims may be needed (CoreBluetooth `maximumWriteValueLength`,
  WinRT `MaxPduSize`, BlueZ `MTU`).
- Interactive approval assumes a console. For service/daemon installs, document `--allow-from` /
  `--allow-all`.
- Firmware must implement port 59 and the new semihost ops (D21, D22) before the hub's default
  handler is useful.
- SYS_ELAPSED source: Windows QPC / `QueryPerformanceFrequency` (u64), `CLOCK_MONOTONIC` in ns
  (10⁹) elsewhere. Because the hub always answers (D31), the device's clock is continuous across
  client connects and disconnects.
