# Plan 05 — Native TUI client (`uclog tui`, macOS / Linux / Windows)

Shared context, protocol and decisions: [README.md](README.md). D# = decisions (README §1),
Q-x = open questions (README §7).

## 1. Goal

A terminal UI in the single `uclog` executable (D13). It connects to **Serial, HID, BLE, UDP
device or a hub (TCP)** and offers the GUI's features **except plotting**. It replaces
`uclog-cli.py` (display/client) and, with `uclog term`, `logcmd.py`. It shares all non-UI crates
with the GUI and hub.

## 2. Technology

| Need | Choice |
|---|---|
| TUI framework | `ratatui` + `crossterm` (Windows Terminal/conhost, macOS, Linux, SSH) |
| Text input | `tui-textarea` (command line, settings fields) |
| File picker | in-TUI browser (`ratatui-explorer` or a small custom widget) with typed-path entry and completion |
| Async | `tokio`; UI loop on the same runtime with an event stream (`crossterm::event::EventStream`) |
| Clipboard | OSC 52, with `arboard` as a fallback |

## 3. Screens

```
┌ uclog ─ STLINK-V3 0042… ─ serial:/dev/ttyACM0 ─ ● live ─ logdata: cache ✓ ─ errors: 0 ┐
│ [1 Logs] [2 Terminal] [3 Commands] [4 Device] [5 FW Update]                           │
├─────────────────────────────────────────────────────────────────────────────────────────┤
│ ts │ lvl │ source │ line │ text           (virtual list, colour by level/source)        │
│ filter: level ≥ INFO · source … · /regex/ · [p]ause · [d]iscard filtered                │
├─────────────────────────────────────────────────────────────────────────────────────────┤
│ terminal (port 61) scrollback + input line                                              │
└ F1 help F2 connect F3 logdata F5 fw-update ^E export ^L clear ^S settings q quit ────────┘
```

- **Status**: live / device not responding (no pulses for more than 2 s) / waiting for hub approval
  / hub busy; error-record counter (README §5.6).
- **Connect** (F2): a single list of Serial/HID/BLE/UDP/hubs (busy shown). `s` scans BLE, `a` adds an
  address, Enter connects. Opens automatically when `-d auto` does not find exactly one device.
- **Logs**: filters as in the GUI, copy, CSV export, max lines. Error records are shown inline.
- **Terminal (61)**: ANSI-coloured scrollback (`ansi-to-tui`), line mode with history, raw-key mode
  (`Ctrl+T`), custom buttons mapped to F6–F12.
- **Commands (0)**: config buttons, `$arg` prompts, pretty CBOR responses.
- **Device**: port 62 info, resolver source, *Load logdata…* (file picker).
- **FW Update**: file picker (`*.bin` + `build/**/zephyr.signed.bin` suggestions + typed path),
  port 59 → 3 fallback, progress gauge, cancel.
- **Settings** (^S): logdata domain/token (keyring)/url, USB filters, files directory (semihost
  sandbox), max lines.
- **Semihost (63)**: always active when connected; sandboxed. Time calls answered by the TUI on direct links, by the hub otherwise (D31).
- **Plots (60)**: not rendered. The status line counts port-60 frames ("plot data: n frames, use
  `uclog gui` or the web version"). `--plot-csv DIR` optionally writes the series to CSV.

## 4. Non-interactive modes

- `--plain`, or automatically when stdout is not a TTY: stream decoded lines
  `ts:level:file:line:text` (today's `uclog-cli.py` format). Error records go to stderr.
- `--json`: one JSON record per line.
- These modes handle semihost, but not the terminal input or command buttons.

## 5. CLI

```
uclog tui  [-d URI|auto] [--baud N] [--usb-filter VID:PID]... [-l FILE]...
           [--cache-dir DIR] [--logdata-domain D] [--logdata-token T] [--logdata-url U]
           [--files-dir DIR] [--plain | --json] [--level LEVEL] [--plot-csv DIR]
           [--config FILE] [-v|-vv]
uclog term [-d URI|auto] [--raw]          # minimal port-61 terminal; replaces logcmd.py, works on Windows
```

## 6. Work breakdown

| # | Task | Size |
|---|---|---|
| 1 | App skeleton: layout, tabs, status bar, key map, config/keyring | S |
| 2 | Connect screen + discovery watcher (shared with GUI) | S |
| 3 | Logs view (virtual list, filters, colours, copy/export, error records) | M |
| 4 | Terminal view (ANSI, line/raw modes, buttons) | S |
| 5 | Commands view | S |
| 6 | Device view + logdata picker | S |
| 7 | FW update view + file picker widget | S |
| 8 | Settings modal | S |
| 9 | `--plain/--json`, `--plot-csv`, `uclog term` | S |
| 10 | Tests: ratatui `TestBackend` snapshot tests with a fake Link; manual matrix (Windows Terminal + conhost, macOS Terminal/iTerm2, Linux, over SSH) | M |

Build this **before** the GUI. It is the smallest complete front end, so it validates the shared
crates (README §6, Phase 3).

## 7. Risks

- Very high log rates: render at most every 50 ms and only draw visible rows. The store is a ring.
- Legacy Windows conhost colour/mouse quirks (crossterm handles most).
- OSC 52 clipboard support varies by terminal (`arboard` fallback, which needs a display on Linux).

## 8. Implementation notes (Phase 3)

- **Code:** in the `uclog` crate. `src/tui/` holds `app.rs` (pure state: keys and session events
  become actions), `ui.rs` (drawing), `logs.rs`, `picker.rs`, `lineedit.rs` and `plotcsv.rs`.
  `src/settings.rs` holds the settings shared with the GUI.
- **Line input and file picker:** a small built-in line editor and file picker replace
  `tui-textarea` and `ratatui-explorer`. This avoids pinning their ratatui versions.
- **Keyring:** `keyring` 3 (macOS Keychain, Windows Credential Manager, Secret Service).
- **Keys:** digits are typed in the Terminal view, so views are also on **Alt+1–5**, and **Esc**
  leaves the Terminal view.
- **Tests:**
  - `src/tui/tests_ui.rs` renders the real UI into `TestBackend`, fed by the loopback device.
  - Pseudo-terminal runs of the release binary on macOS, against loopback, the NUCLEO, and the
    NUCLEO through a hub.
  - Still to do: the manual Windows/Linux/SSH matrix ([Plan 07](07-release.md)).
