# Web workspace (TypeScript)

Phases 5 and 6 of [docs/plans](../docs/plans/README.md): the browser client from
[plan 02](../docs/plans/02-web.md), as the embeddable `<uc-log>` element and a standalone page, and
the VS Code extension from [plan 03](../docs/plans/03-vscode.md). The browser client connects over
Web Serial, WebHID, Web Bluetooth, or a WebSocket to a `uclog hub`, and has a simulated device for
trying it without hardware.

| Package | Contents |
|---|---|
| `@uclog/core` | CBOR, COBS, frame scanner, 2-byte peek, mux; logdata (CBOR or ELF `.logdata_cbor`) and the C-printf decoder; semihost with a pluggable file system; FW update (59 → 3); logdata resolver (pluggable cache and fetch); the session's transport and decode halves; log store, plot model; the simulated `loopback:` device. No DOM |
| `@uclog/transports-web` | `SerialLink`, `HidLink`, `BleLink` (main thread), `WebSocketLink` (busy and approval states), `DirectoryFs` (semihost files in a folder the user granted) |
| `@uclog/workers` | the transport and decode workers, their message protocol, `Client` (the main-thread side), and the host services an embedder can provide (`rpc.ts`) |
| `@uclog/ui` | Lit components: `uc-log`, `uc-device-picker`, `uc-log-view`, `uc-terminal`, `uc-commands`, `uc-plots`, `uc-device-info`, `uc-fw-update`, `uc-settings` |
| `@uclog/element` | the `<uc-log>` bundles: ESM and IIFE with the workers inlined, plus the workers as files for `worker-base` |
| `apps/demo` | the standalone page |
| `apps/vscode` | the VS Code extension ([README](apps/vscode/README.md)): the logger in a webview, devices through a private `uclog-hub` |

```sh
npx -y pnpm@12.10.1 install
npx -y pnpm@12.10.1 dev          # the demo page on http://localhost:5173
npx tsc -p tsconfig.json         # type check
npx vitest run                   # unit tests, spec/vectors, worker hosts
npx playwright test              # end-to-end in Chrome (the hub test needs native/target/*/uclog-hub)
npx -y pnpm@12.10.1 build        # packages/element/dist and apps/demo/dist
```

The VS Code extension (in `apps/vscode`):

```sh
npx tsc -p apps/vscode/tsconfig.json
(cd apps/vscode && npx vite build --mode host && npx vite build)   # dist/extension.cjs, dist/webview
(cd apps/vscode && node scripts/package.mjs --local)              # bin/ from native/target, then uclog-0.1.0.vsix
npx playwright test -c apps/vscode/playwright.config.ts           # in the installed VS Code (UCLOG_VSCODE=…)
UCLOG_VSIX=apps/vscode/uclog-0.1.0.vsix npx playwright test -c apps/vscode/playwright.config.ts
UCLOG_TEST_DEVICE=serial:/dev/cu.usbmodem… npx playwright test -c apps/vscode/playwright.config.ts hw
```

Hardware tests in Chrome: `UCLOG_TEST_DEVICE=… UCLOG_TEST_ELF=… npx playwright test hw` (through
the hub), and `UCLOG_TEST_BLE="ucLog test STM32WBA55G-DK1" UCLOG_TEST_ELF=../firmware/stm32wba55g-dk1/build/app.elf
npx playwright test ble` (Web Bluetooth, the chooser answered through DevTools).

To try it by hand, build it and fill `bin/` (the first three lines), then open `apps/vscode` in VS
Code and press F5 (*Run Extension*), or install the VSIX
(`code --install-extension apps/vscode/uclog-0.1.0.vsix`).

The demo page's query parameters set the element's attributes, for example
`?hub-url=ws://lab-pc:9000&theme=dark&layout=grid`, and `?connect=loopback:rate=5` connects at
once. Web Serial, WebHID and Web Bluetooth need Chrome or Edge and a secure context (`localhost`
counts). On Windows, Web Bluetooth is as fast as the Bluetooth driver allows: keep it current
(firmware/README.md, "BLE throughput with Windows 11").

## Using it

- **Connect…** lists the USB devices the site may already use, the browser's choosers for Serial,
  HID and Bluetooth, hub URLs (recent ones are remembered; browsers cannot browse mDNS), and the
  simulated device.
- A serial or HID device that is unplugged or resets is opened again when it comes back.
- **Views**, as tabs or a grid (⋯ menu):
  - **Logs**: a virtualised table. Filters: level, source, line, regex search; *Discard
    filtered*. Click, ⌘/Ctrl-click and Shift-click select rows, ⌘/Ctrl-C copies them.
    Double-click a row to show only its source; right-click for more filters, copy, and the
    source's colour. Error records are always shown; the status bar counts them.
  - **Terminal** (port 61): xterm.js with an input line and history, the configured buttons, and a
    character mode. Semihost stdout/stderr appear here; a stdin read takes the next line.
  - **Commands** (port 0): buttons with their `$arg` fields; a free line (text, or a JSON object
    sent as CBOR).
  - **Plots** (port 60): one tab per plot. Line plots with up to three y-axes: *Follow* fits the
    data, drag zooms, double-click fits again; click sets cursor A, Shift-click cursor B (Δx and
    1/Δx), ⌘/Ctrl-click adds a marker. Heatmaps and spectrograms with a colour bar.
  - **Device**: port-62 fields, the logdata and where it came from, *Load logdata…* (`.logdata`
    or ELF), *Serve files from folder…* for semihost (what the device writes is in the folder
    within 0.2 s, not only when it closes the file), counters and notes.
  - **FW update**: the signed `.bin`, port 59 then 3, progress, cancel.
- **Export all** (⋯) downloads a zip: the logs as CSV, the terminal text and every plot as CSV.
- **Settings** (⋯): logdata domain, URL and token, max log lines, lines sent on connect, the
  button lists (JSON, the same schema as the native GUI). Settings are in `localStorage` under
  the `storage-key`. The token is kept for the session unless *Remember* is ticked (IndexedDB).
- **Logdata** is found by the device's `app_hash`: files loaded manually, then the IndexedDB
  cache, then the logdata service. A file loaded manually whose name is the device's
  `app_hash` (`<app_hash hex>.logdata`) is saved in the cache, so the next connection finds it;
  other files are used for the session only.

A runnable example page (both bundles, a strict CSP, the element's methods and events):
[examples/embed](examples/embed/README.md).

## Embedding in an application

An application that is more than a web page (the VS Code extension) sets `UcLog.host` before the
element starts. It can provide its own Connect UI, file pickers, saving and settings, and services
the workers call through the main thread: a logdata cache, a fetch without CORS, logdata from local
builds, and semihost files. A link the host owns is passed as `connect({kind: 'external', link})`.

## Embedding

```html
<script type="module" src="uclog-element.js"></script>
<uc-log logdata-domain="acme.example" hub-url="ws://localhost:9000"
        panels="log,terminal,commands,plots,fw-update" layout="tabs" theme="auto"></uc-log>
```

| | |
|---|---|
| Attributes | `logdata-domain`, `logdata-url`, `hub-url`, `panels` (`log,terminal,commands,plots,device,fw-update`), `layout` (`tabs`, `grid`), `theme` (`auto`, `light`, `dark`), `storage-key`, `worker-base`, `emit-logs` |
| Property | `logdataToken` |
| Methods | `connect(spec)` (`ws://host:9000`, `loopback:`, `serial`/`hid`/`ble` to open the chooser, or a `SerialPort`/`HIDDevice`/`BluetoothDevice`), `disconnect()`, `send(port, bytes or text)`, `loadLogdata(file or {name, data})`, `ready()` |
| Events | `uc-ready`, `uc-connect`, `uc-disconnect`, `uc-device-info`, `uc-log` (with `emit-logs`; `detail.records`), `uc-error` |
| Styling | `--uc-*` custom properties (colours, `--uc-font`, `--uc-mono`, `--uc-radius`) and `::part()` (`header`, `tabs`, `main`, `panel`, `panel-<name>`, `status`, `toolbar`, `table`, `terminal`, `plot`) |

Host requirements:

- A secure context. In an iframe: `allow="serial; hid; bluetooth"`.
- The inlined workers need `worker-src blob:` in the page's CSP. Without it, serve
  `dist/workers/*.js` and set `worker-base` to their directory.
- A hub on another machine needs `wss://`. A local hub asks its operator about pages that are
  not on its `--allow-origin` list (`localhost` is allowed by default).
- The logdata service only answers the origins it allows (CORS).

Sizes (gzip): `uclog-element.js` 135 KB, plus 29 KB for uPlot, loaded with the first line plot.
The IIFE build is one file of 158 KB.

## Design notes

- **Threads** (plan 02 §3):
  - The **transport worker** owns the session's transport half: framing, pulses and the wake gap,
    liveness, semihost replies, FW update. WebSocket links and the simulated device run there.
  - The **decode worker** selects logdata, decodes logs, keeps the log store and its filter
    index, and models the plots.
  - The **main thread** only draws. It asks for the rows in view and for plot data decimated to
    the plot's width.
  - Serial, HID and Bluetooth devices live on the main thread, which forwards raw chunks without
    looking at them.
- **Back-pressure:** the transport worker sends log frames to the decode worker in acknowledged
  batches. With 64 batches unacknowledged it drops log frames and then reports how many, as a
  `decoder_behind` error record. Port 63, 59 and 61 traffic is never dropped.
- **Measured:** 20 000 logs decoded in 0.11 s (about 175 000 logs/s) with no main-thread long
  tasks; on the NUCLEO-L452RE-P through the hub, the firmware's `test` lines and a 5000-log burst
  decoded with no error records.
