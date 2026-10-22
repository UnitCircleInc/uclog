# Plan 04 — Native graphical client (`uclog gui`, macOS / Linux / Windows)

Shared context, protocol and decisions: [README.md](README.md). D# = decisions (README §1),
Q-x = open questions (README §7).

## 1. Goal

A new Rust GUI in the single `uclog` executable (D13). It replaces `uclog-gui.py`, matches its
features, and connects to **Serial, HID, BLE, UDP device or a hub (TCP)**. It shares
`uclog-proto/-logdata/-semihost/-link/-session` with the TUI and hub. **No DearPyGui code is reused.**
The existing Python GUI serves only as a feature reference.

## 2. Technology

| Need | Choice |
|---|---|
| Windowing/rendering | `eframe` (egui), `glow` backend (smaller than wgpu; OpenGL 3.x / ES works on the Pi) |
| Dockable panels / layout persistence | `egui_dock` (tabs that can be split or floated); layout saved via eframe storage |
| Log table | `egui_extras::TableBuilder` (virtualised rows) |
| Plots | `egui_plot`: lines, cursors and markers, `PlotImage` texture for heatmap/spectrogram |
| Terminal | `vte` parser → simple cell buffer widget (VT100 subset: colours, cursor-forward, erase line) |
| File dialogs | `rfd` (native: Cocoa, Win32, xdg-desktop-portal on Linux, so no GTK dependency) |
| Async runtime | `tokio` on a background thread. The Session posts view updates over channels and calls `ctx.request_repaint()` |
| Settings / secrets | `serde_json` in the platform config dir (`directories`); token in the OS keyring (`keyring`) |
| Screenshot | `ViewportCommand::Screenshot` → PNG (`image` crate) |

## 3. Feature parity (from the current GUI)

| Area | Requirement |
|---|---|
| **Connect** | One *Connect* dialog: Serial / HID / BLE (*Scan* button) / mDNS hubs (busy shown) / *Add address…* (tcp/udp); columns kind, name, serial, description; filter box. `-d auto` connects when there is exactly one match. Status shows connected / device not responding (no pulses for more than 2 s) / waiting for hub approval / hub busy |
| **Log view** | level filter, source & line filters (right-click / double-click to filter like today), text search, per-source background colours (persisted), auto-scroll, *discard filtered*, copy row(s), CSV export, max lines, error records with counters (README §5.6) |
| **Terminal (61)** | scrollback, input line, history (persisted, max 50), custom buttons from config, copy, export, clear; sends `version s` and `help` on connect (as today; configurable) |
| **Commands (0)** | custom buttons with CBOR map or string cmds, `$arg` input fields, CBOR responses pretty-printed |
| **Plots (60)** | create/stream/clear per port-60 messages: line plots with up to 3 y-axes (`yser/y2ser/y3ser`, limits, props, `xscroll`), heatmap, spectrogram, cursors and markers, auto/manual fit, CSV export, one dock tab per plot |
| **Device info (62)** | board, image type, app_hash, resolver source (manual/cache/remote/sibling), logdata file(s) |
| **Logdata** | *Load ELF/logdata…* (rfd), resolver per README §5.3, per-device memory of manual selections (replaces `target_info_archive`) |
| **FW update** | rfd file picker (`.bin`) + suggestions from scanning `build/**/zephyr.signed.bin` under the CWD; port 59 → 3 fallback; progress, cancel; on success, seeds the resolver |
| **Semihost (63)** | always active when connected (directly, or via a hub, D3). Sandbox root = *Settings → files directory* (D15). Time calls answered by the GUI on direct links, by the hub otherwise (D31) |
| **File menu** | Load logdata, Screenshot, Set export directory, Open export directory, Export all (logs + terminal + plots with a prefix), Exit |
| **View** | UI scale (replaces the font-size menu), light/dark theme, reset layout |
| **Settings dialog** | logdata domain/token/url, USB filters, max log lines, files directory, export directory, buttons editor (or *Open settings file*) |

The GUI does **not** run an in-process hub (single-client hubs, README §4). To share a device, run
`uclog hub` and connect the GUI to it.

## 4. Structure

```
uclog-gui/
  app.rs          eframe App: dock state, menus, status bar
  connect.rs      Connect dialog (uses uclog-link discovery watcher)
  views/log.rs    table + filters  (data from session's log store)
  views/term.rs   terminal widget
  views/cmds.rs   commands
  views/plots.rs  plot manager + plot types
  views/info.rs   device info / logdata
  views/fw.rs     FW update
  settings.rs     config schema + dialog; first-run import of ~/ucLogger.json (buttons, colours)
```

High log rates: the session thread decodes and appends to a ring store with an epoch counter. The UI
reads only the visible rows each frame. Plot series are decimated per pixel column.

## 5. CLI

```
uclog gui [-d URI|auto] [--baud N] [--usb-filter VID:PID]... [-l FILE]...
          [--cache-dir DIR] [--logdata-domain D] [--logdata-token T] [--logdata-url U]
          [--files-dir DIR] [--config FILE] [-v|-vv]
```

## 6. Packaging

- Part of the `uclog` release build (README §4): Linux x64/arm64/armv7 (AppImage + tarball),
  macOS arm64/x64 (`.app` in a `.dmg` + plain binary; `NSBluetoothAlwaysUsageDescription`; signed
  and notarised), Windows x64/arm64 (signed `.exe`; optional MSI).
- Size target ≤ 15 MB stripped (`opt-level="z"`, LTO, `panic="abort"`, `strip`).
- Linux: udev rules for hidraw/tty access shipped in the tarball, with an install hint at first run if
  access is denied.

## 7. Work breakdown

| # | Task | Size |
|---|---|---|
| 1 | App shell: eframe, dock layout, menus, status bar, settings load/save, keyring | M |
| 2 | Session integration (tokio thread ↔ UI channels, repaint), log ring store | M |
| 3 | Connect dialog + discovery watcher | M |
| 4 | Log view (table, filters, colours, copy/export, error records) | M |
| 5 | Terminal widget (vte) + buttons/history | M |
| 6 | Commands view | S |
| 7 | Plots: lines (3 y-axes), cursors/markers, heatmap/spectrogram, export | L |
| 8 | Device info + logdata loading + resolver UI | S |
| 9 | FW update view | S |
| 10 | Screenshot, export all, export dir | S |
| 11 | Packaging (dmg/notarise, AppImage, signed exe) | M |
| 12 | Tests: view-model unit tests, replay test feeding a recorded wire capture (logs + port 60/61/62) through a fake Link; manual matrix 3 OS × Serial/HID/BLE/UDP/hub | M |

## 8. Risks

- **Multiple y-axes**: `egui_plot` has one coordinate system per plot. Secondary axes are done by
  normalising series into the primary range and drawing custom axis labels for y2/y3. Prototype
  first, because the current GUI relies on this.
- Heatmap/spectrogram update rates: stream into a ring texture with partial updates.
- Pi GPUs: `glow` with GLES 3.0; test on Pi 4/5.
- egui looks less "native" than Qt. That is acceptable for an engineering tool, and keeps the
  executable small and self-contained.
