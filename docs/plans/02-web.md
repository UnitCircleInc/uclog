# Plan 02 — Web version (Chrome)

Shared context, protocol and decisions: [README.md](README.md). D# = decisions (README §1),
Q-x = open questions (README §7).

## 1. Goal

A browser client with GUI-level features (log view, terminal, commands, plots, device info, FW
update, semihost) that connects over:

- **Web Serial**: default filters STLINK, FTDI, Zephyr CDC `2FE3` (D1)
- **WebHID**: VID 0x2FE3; reports 0xA1/0xA2, 63-byte chunks
- **Web Bluetooth**: service `8683b8f3-…`; automatic chunking (D18)
- **WebSocket** to a `uclog hub`: same port as TCP, default `ws://localhost:9000`; `wss://` for
  remote hubs (D9, D10)

It ships as an **embeddable component** (`<uc-log>`) for third-party sites, plus a standalone demo.
Its code is shared with the VS Code extension (Plan 03).

## 2. Packages (pnpm workspace `web/`)

| Package | Contents |
|---|---|
| `@uclog/core` | COBS, FrameScanner + 2-byte peek, mux, ports, `Session` (pulse, liveness, dispatch), logdata loader (CBOR + ELF `.logdata_cbor`), printf formatter, FW-update client (port 59 → 3 fallback), logdata resolver (pluggable cache/fetch), semihost (pluggable `FsProvider`; time ops built in), error records (README §5.6) |
| `@uclog/transports-web` | `SerialLink`, `HidLink`, `BleLink`, `WebSocketLink`; discovery helpers |
| `@uclog/workers` | the two worker entry points (§3) and the typed message protocol between them |
| `@uclog/ui` | Lit components: `uc-device-picker`, `uc-log-view`, `uc-terminal`, `uc-commands`, `uc-plots`, `uc-device-info`, `uc-fw-update`, `uc-settings` |
| `@uclog/element` | `<uc-log>` bundle (ESM + IIFE), workers inlined as blob URLs or emitted alongside |
| `apps/demo` | static page hosting `<uc-log>` |

**WASM option (D28):** modules in `@uclog/core` sit behind interfaces, so the scanner, decoder or
semihost codec can later be swapped for the Rust crates compiled to WASM (README §4.1). The shared
vectors decide each swap. v1 is pure TypeScript.

Tooling: TypeScript strict, Vite library mode, Vitest (+ `spec/vectors`), Playwright (fake Link),
Changesets. Dependencies: `lit`, `cbor-x`, `xterm.js`, `uPlot` (plots lazy-loaded).

## 3. Threading model (high log rates)

Decoding and presentation run on a **different worker from transport processing**, and the main
thread only renders:

```
 main thread                         Transport worker                       Decode/Presentation worker
 ─────────────                       ────────────────                       ──────────────────────────
 Web Serial/HID/BLE objects  ──raw──► FrameScanner, pulse timer,   ──frames──► logdata decode, error records,
 (or streams transferred)   ◄─bytes── liveness, port routing,                 filter/search index, row model,
 WebSocket (may live in worker)       FW update, semihost,                    plot series aggregation,
 UI components ◄────────────── view diffs (batched per rAF) ◄──────────────── CSV/zip export
```

- **Transport worker** owns the Session: framing, 0.5 s pulses, 2 s liveness, port routing, FW
  update state machine, semihost replies (time-critical: the device waits on them), port-61
  terminal bytes. It must never block on decoding.
  - `WebSocket` is opened **inside** the transport worker.
  - Web Serial: transfer `port.readable`/`port.writable` to the worker (transferable streams), or
    open the port in the worker where Chrome exposes `navigator.serial` to dedicated workers.
  - WebHID / Web Bluetooth stay on the main thread, which forwards raw chunks as transferred
    `ArrayBuffer`s with no processing.
- **Decode/presentation worker**: receives log frames and port-60/62 payloads over a dedicated
  `MessageChannel` from the transport worker (not via the main thread). It does all logdata
  decoding, keeps the log store (ring of `max_lines`), applies filters, and computes the **visible
  window** for the virtualised table and downsampled plot series.
- **Main thread**: DOM/canvas only. It requests windows (`rows 1200–1260 under filter F`) and
  applies batched updates once per animation frame.
- Back-pressure: if the decode worker lags, the transport worker coalesces log frames into larger
  batches. It never drops port 63/59/61 traffic. Log frames are dropped only beyond a cap, with an
  error record.

## 4. Core port (Python → TypeScript)

| Python | TypeScript | Notes |
|---|---|---|
| `uccobs` | `cobs.ts` | reuse vectors |
| `CobsDecode/MuxDecode/chain` | `FrameScanner`, `Session` | events, not callable chains |
| `logdata.load_from_cbor`, `parse_*`, `cfmt2pfmt`, `LogData.decode` | `logdata.ts`, `printf.ts` | **C-printf semantics; exact Python `%` parity is not required.** `{sym}`, `{enum:…}`, `ll`, `%p`, 64-bit via `BigInt`. Golden tests check "reasonable" output, with snapshots reviewed once |
| `load_cbor_from_elf` | `elf.ts` | minimal ELF32 section reader for `.logdata_cbor` |
| `Semihost` | `semihost.ts` | time ops answered on direct links (Web Serial/HID/BLE). Via a hub, the hub answers them (D31). File ops through `FsProvider`. With no folder granted → error replies (`ENOSYS`/`EACCES`). newlib errno numbers |
| `UnitUpdate._thread_update` | `fwUpdate.ts` | port 59 first, 1 s probe, fallback to port 3 nested format (README §3.4); `AbortSignal` cancel |
| `app_hash_handler` | `resolver.ts` | manual → IndexedDB cache → `fw.unitcircle.ca` → (if the user picked a `.bin`) sibling match |

Decoded record: `{seq, ts, level, file, line, text, target, addr, error?}`. Timestamps =
`performance.now()` relative to the session start (D16).

## 5. Transports

| Link | API | Details |
|---|---|---|
| Serial | `navigator.serial.requestPort({filters})`, `open({baudRate: 1_000_000})` | `getPorts()` re-lists granted ports. STLINK split. On `disconnect`, reopen when the port reappears |
| HID | `navigator.hid.requestDevice({filters:[{vendorId:0x2FE3}]})` | `sendReport(0xA2, chunk≤63, zero-padded)`; `inputreport` with id 0xA1 |
| BLE | `navigator.bluetooth.requestDevice({filters:[{services:[UUID]}]})` | notifications in; writes of whole frames up to 512 bytes per `writeValue*` (Chrome negotiates MTU / long writes). **No chunk-size setting** (D18). Pairing via the OS |
| Hub | `WebSocket(url)` in the transport worker | default `ws://localhost:9000`, recent URLs saved. Close code `4001` → "hub busy". Open but silent → "waiting for hub approval…" (hub prompt, D14), 60 s timeout |

Liveness on every link: send a pulse every 0.5 s while connected, and send a pulse + wait ≥ 50 ms
before the first frame (UART wake, README §3.1). No bytes from the device for more than 2 s →
"device not responding", shown without disconnecting.

Device picker: one list with *Granted USB devices* (Serial/HID), *Add device…* (Chrome
chooser for Serial/HID/BLE), and *Hubs* (recent/entered URLs). Browsers cannot browse mDNS.

## 6. UI features

| Feature | Web implementation |
|---|---|
| Log table: level/source/line/text filters, per-source colours, copy, CSV export, max lines | virtualised table fed by the decode worker; error records (README §5.6) with a status-bar counter |
| Terminal (61) | `xterm.js`, history, custom buttons |
| Commands (0) | same JSON button schema incl. `$arg` inputs |
| Plots (60) | uPlot lines (3 y-axes, cursors, auto/manual fit); canvas heatmap/spectrogram; CSV export |
| Device info | port 62 fields, resolver source, hub info when connected via a hub |
| FW update | `<input type=file accept=".bin">` / `showOpenFilePicker`; progress; cancel; shows port used (59/3) |
| Load logdata | file picker (`.logdata`, `.elf`) |
| Semihost (63) | always active when connected (directly, or via a hub where connection = opt-in, D3). *Serve files from folder…* grants a directory (File System Access API); without it, file ops return errors |
| Export all | zip: logs CSV + terminal text + plot CSVs (replaces the screenshot feature; the browser can do screenshots) |

Settings: logdata domain/token/url, max log lines, buttons, colours. `localStorage` (namespaced by
`storage-key`). The token is per session unless the user chooses *remember* (IndexedDB).

## 7. Embedding contract (`<uc-log>`)

```html
<script type="module" src="https://cdn.example/uclog-element.js"></script>
<uc-log logdata-domain="acme.example" hub-url="ws://localhost:9000"
        panels="log,terminal,commands,plots,fw-update" theme="auto"></uc-log>
```

- Attributes: `logdata-domain`, `logdata-url`, `hub-url`, `panels`, `layout` (`tabs|grid`), `theme`,
  `storage-key`, `worker-base` (where worker files are served from, if not inlined).
  `logdataToken` is a property only.
- Events: `uc-connect`, `uc-disconnect`, `uc-device-info`, `uc-log` (opt-in `emit-logs`), `uc-error`.
- Methods: `connect(linkSpec)`, `disconnect()`, `send(port, bytes)`, `loadLogdata(file)`.
- Styling: CSS custom properties (`--uc-*`) and `::part()`.
- Host requirements:
  - Secure context.
  - In an iframe: `allow="serial; hid; bluetooth"`.
  - The site's CSP must allow `worker-src blob:` (or `worker-base` must be set).
  - fw.unitcircle.ca CORS allows only the account's own or registered origins unless token-less
    reads are enabled.
  - A LAN hub needs `wss://`.
  - A local hub prompts for unknown origins (D14). Hub operators can pre-allow with
    `--allow-origin https://site.example`.

Distribution: npm `@uclog/element` + versioned single-file build on GitHub Releases/CDN. Target
≤ 250 KB gz, excluding the lazy-loaded plots.

## 8. Work breakdown

| # | Task | Size |
|---|---|---|
| 1 | Workspace, CI, vectors runner | S |
| 2 | Core: COBS/scanner/peek/mux/Session (pulse, liveness) | S |
| 3 | logdata loader + printf (C semantics) + error records | M |
| 4 | Resolver + IndexedDB cache + fw.unitcircle.ca client | S |
| 5 | Worker split: transport worker, decode/presentation worker, MessageChannel protocol, back-pressure | M |
| 6 | Links: Serial (stream transfer), HID, BLE, WS (busy/approval states) + fake Link | M |
| 7 | UI: picker, virtualised log view, terminal, commands, device info | M |
| 8 | Plots | L |
| 9 | FW update (59→3) + semihost (time ops + FS provider) | M |
| 10 | `<uc-log>` element, theming, docs, demo | M |
| 11 | Playwright E2E with fake Link; throughput test (≥ 10k logs/s without UI jank); hardware matrix (Chrome × Win/macOS/Linux × Serial/HID/BLE/WS) | M |

## 9. Risks

- Web Bluetooth on Linux may need a Chrome flag. Pairing is OS-driven.
- WebHID refuses protected usage pages. **Check that the firmware HID descriptor uses a
  vendor-defined usage page.**
- Whether Web Serial/WebHID objects can run in workers depends on the Chrome version. The design
  works either way (main thread forwards, or streams are transferred).

## 10. Implementation notes (Phase 5)

Implemented in [`web/`](../../web/README.md). Where it differs from the plan above:

- **Packages and tools:**
  - pnpm 12 (run with `npx pnpm@12.10.1`), TypeScript 5.9, Vite 8, Vitest 5, Playwright 1.64,
    Lit 3.3, xterm.js 6, uPlot 1.6.
  - **No `cbor-x`**: `@uclog/core` has its own small CBOR codec, like the Python library. Logdata
    needs array map keys, and the vectors compare encoded bytes.
  - Workspace packages export their TypeScript sources; only `@uclog/element` is built.
  - Changesets and npm publishing are not set up yet (Plan 07).
- **Threads:**
  - **Web Serial stays on the main thread**, like WebHID and Web Bluetooth. It forwards raw
    chunks and does nothing else. Transferring `port.readable`/`writable` to a worker would leave
    the port impossible to close.
  - Back-pressure: log frames go to the decode worker in acknowledged batches. With 64 batches
    outstanding, log frames are dropped and then reported as one `decoder_behind` error record
    (a new kind; protocol.md §7 lists the others).
- **Session:**
  - The transport half (`TransportSession`) and decode half (`DecodeSession`) are separate
    classes in `@uclog/core`, so they run in the two workers and also together in unit tests.
  - Direct links ask for the device info (F7) and hold log frames for up to 1 s for the reply,
    like the native session.
- **Resolver:**
  - Manual files, then IndexedDB, then the logdata service.
  - The "sibling `.bin`" step is left out: a page cannot see the files next to one the user
    picked.
  - A file loaded manually is saved in the cache under the device's `app_hash`.
  - The demo page loads `spec/vectors/sample.logdata` under the simulated device's `app_hash`
    (`5a…5a.logdata`), so it never applies to real devices.
- **Semihost files:** *Serve files from folder…* uses the File System Access API. A writable stream
  only reaches the file when it is closed, so a file open for writing is held in memory, shared by
  all its handles (reads see the writes), and written to the file 200 ms after a write and on
  close. Closing the page loses at most the last 200 ms; closing it mid-write leaves the previous
  copy (Chrome swaps the file in whole).
- **Simulated device:** `loopback:` in TypeScript, matching the native one (same commands and
  plots). It runs in the transport worker.
- **Element:**
  - `<uc-log>` starts its workers a microtask after it is connected. A page that has the element
    before the bundle's script would otherwise upgrade it before the bundle sets its inlined
    workers.
  - `ready()` and a `uc-ready` event were added for that.
- **Tests:**
  - **Vitest (76):**
    - every `spec/vectors` file;
    - the session halves against the simulated device: decoding, the cache, semihost incl.
      stdin, plots, oversize sends;
    - FW update on 59, fallback to 3, cancel;
    - the semihost sandbox, the resolver (fetch, token, problem details, 404), printf, the log
      store;
    - the worker hosts over a real `MessageChannel`, incl. back-pressure;
    - button payloads, the zip writer.
  - **Playwright (Chrome):**
    - every view with the simulated device;
    - settings and buttons;
    - both built bundles on a third-party page (IIFE with `worker-src blob:` only; ESM with
      `worker-base`);
    - the native hub over WebSocket, incl. "hub busy";
    - throughput and follow/scroll behaviour.
  - **Hardware:** `e2e/hw.spec.ts`, the device through the native hub.
- **Measured:**
  - 20 000 logs in 0.11 s (about 175 000 logs/s), no main-thread long tasks.
  - Bundle: 135 KB gzip, plus 29 KB for uPlot when the first line plot opens.
  - On the NUCLEO-L452RE-P through the hub: the firmware's `test` lines (every argument kind)
    and a 5000-log burst decoded with no error records.
- **Not done yet** ([Plan 07](07-release.md)):
  - Web Serial, WebHID and Web Bluetooth with real devices: the choosers cannot be automated, so
    this needs a manual run.
  - Chrome on Windows and Linux.
  - npm publishing of `@uclog/element` and a versioned CDN build.
