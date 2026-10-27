# Native workspace (Rust)

Phases 1–4 of [docs/plans](../docs/plans/README.md): the shared crates, the `uclog`
subcommands, the hub, the terminal UI and the GUI.

| Crate | Contents |
|---|---|
| `uclog-proto` | COBS, frame scanner, 2-byte peek, mux, CBOR helpers, CRC-32C, FW-update messages, semihost codec, link chunking. No I/O (WASM-friendly) |
| `uclog-logdata` | `.logdata` / ELF `.logdata_cbor` loader, log decoding, C-printf formatter |
| `uclog-semihost` | sandboxed port-63 handler, newlib errno mapping, host clock sources (QPC / `CLOCK_MONOTONIC`) |
| `uclog-link` | Serial, HID, BLE, UDP (Don't Fragment), TCP and WebSocket links; the simulated `loopback:` device; device URIs; USB/HID/BLE/mDNS discovery |
| `uclog-session` | pulses and wake gap, liveness, dispatch, logdata resolver, semihost bridge, FW update (59 → 3) |
| `uclog-hub` | the hub ([plan 01](../docs/plans/01-hub-server.md)): backlog ring, device task, raw/WebSocket/TLS on one port, UDP clients, admission, port-63 routing, mDNS, local registry. Library plus the small `uclog-hub` executable |
| `uclog` | the executable: `list`, `decode`, `term`, `tui`, `gui`, `fw-update`, `hub`; settings shared by the front ends. `--no-default-features` leaves out the GUI (no windowing dependencies) |

## Setup

macOS: the toolchain comes from Homebrew's `rustup`, and the repository's `.envrc` puts it on
`PATH` with [direnv](https://direnv.net):

```sh
brew install rustup direnv
rustup default stable            # installs the stable toolchain
echo 'eval "$(direnv hook zsh)"' >> ~/.zshrc   # once (bash: direnv hook bash)
# then, in the repository root:
direnv allow
```

Homebrew's `rustup` is keg-only, so without direnv `cargo` is not found unless
`$(brew --prefix rustup)/bin` is on `PATH`. `.envrc` also loads a git-ignored `.secrets` if there
is one (for example `export UCLOG_LOGDATA_TOKEN=…` for testing; see
[Logdata service settings](#logdata-service-settings)).

Other platforms: install the stable toolchain with [rustup](https://rustup.rs); `.envrc` is not
needed there. Linux also needs the packages under [Platforms](#platforms). On Windows, don't use
direnv ([Windows](#windows)).

## Build and test

```sh
cargo test                       # unit tests, spec/vectors, fake-device session tests, hub tests, CLI tests
cargo clippy --all-targets
cargo build --release            # target/release/uclog and target/release/uclog-hub

# Optional: decode every format in a directory of .logdata files
UCLOG_LOGDATA_CORPUS=~/.cache/uclog cargo test -p uclog-logdata --release -- --ignored corpus
```

```sh
uclog list [--json] [--ble-scan 5]                 # devices, local hubs (registry) and remote hubs (mDNS)
uclog-hub list --json                              # local USB devices only (for the Python library)
uclog term -d serial:/dev/ttyACM0 [--logs] [--raw] [--port 0] [-c "version s" --exit-after 2]
uclog decode -l build/zephyr/zephyr.elf capture.bin
uclog fw-update -d auto build/zephyr/zephyr.signed.bin
```

`-v` logs info, `-vv` adds a hex trace of every byte sent and received (`uclog::wire`).

## Logdata service settings

For everyday use, set the logdata service in the **TUI or GUI settings**: the domain and, if it
isn't fw.unitcircle.ca, the URL. They are saved in `settings.json` and used by every `uclog`
command. The token is entered there too and kept in the OS keyring (macOS Keychain, Windows
Credential Manager, Linux Secret Service), not in a file. The VS Code extension has its own
settings (`uclog.logdata.*`, and **ucLog: Set Logdata Token**).

Environment variables are for the **command line and automated testing** (scripts, CI, pytest).
The Python library reads the same `UCLOG_LOGDATA_*` and `UCLOG_CACHE_DIR`:

| Variable | Same as |
|---|---|
| `UCLOG_LOGDATA_DOMAIN` | `--logdata-domain` |
| `UCLOG_LOGDATA_TOKEN` | `--logdata-token` |
| `UCLOG_LOGDATA_URL` | `--logdata-url` |
| `UCLOG_CACHE_DIR` | `--cache-dir` (default `$XDG_CACHE_HOME/uclog`, else `~/.cache/uclog`) |
| `UCLOG_DEVICE` | `-d` |

Which wins: the command line, then the environment, then the settings; the keyring token is used
only when neither gives one.

- macOS, Linux: `export UCLOG_LOGDATA_DOMAIN=example.com` in the shell, the CI job or a
  git-ignored `.secrets` that `.envrc` loads.
- Windows: `$env:UCLOG_LOGDATA_DOMAIN = "example.com"` for the current PowerShell, or
  `setx UCLOG_LOGDATA_DOMAIN example.com` to keep it for your account (it reaches only windows
  opened afterwards). `setx` stores a token in plain text in the registry; on a desktop, use the
  keyring through the settings instead.

## GUI

```sh
uclog gui                                          # connects when exactly one device matches, else opens Connect
uclog gui -d tcp://lab-pc:9000 -l build/zephyr/zephyr.elf --files-dir ./files
uclog gui -d loopback: -l spec/vectors/sample.logdata   # simulated device; type `plot` in the Terminal
```

- **Views**, as dock tabs (drag to split, float or rearrange; the layout is remembered):
  - **Logs**, **Terminal**, **Commands**, **Device**, **FW Update**.
  - **One tab per plot** the device creates on port 60. New plot tabs open in the background.
- **Logs:**
  - Filters: level, source, line and regex search.
  - Follow, *Discard filtered*, copy and CSV export.
  - Click, ⌘/Ctrl-click and Shift-click select rows.
  - Double-click a row to show only its source. Right-click it for more filters, copy, and the
    source's background colour.
  - Colours are saved per source; ones from the legacy GUI are imported.
  - Error records are always shown, with counters.
- **Terminal:**
  - ANSI colours, scrollback, an input line with history, and the configured buttons.
  - Semihost stdout and stderr appear here, and stdin reads take the next line.
- **Commands:**
  - Buttons from the settings, with their `$arg` fields beside them.
  - A free line: text, or a JSON object sent as CBOR.
  - CBOR replies are shown readably.
- **Plots:**
  - Line plots with up to three y-axes, labelled on the right.
  - `scatter`, `digital` and `infinite` series.
  - *Fit* follows the data; panning or zooming stops it, and double-click resets the view.
  - Two draggable **cursors** with Δx, 1/Δx and values.
  - ⌘/Ctrl-click to drop a **marker**.
  - Heatmaps and spectrograms; CSV export.
- **Device:** device info, logdata source, frame and error counters, and resolver notes.
  *Load logdata…* also saves the file in the cache under the device's `app_hash`, so it is found
  next time.
- **FW Update:** pick an image (`*.signed.bin` under the working directory are suggested, unless
  that is the home directory or above); then progress and cancel.
- **File menu:** Connect (⌘K), Load logdata (⌘L), Screenshot (PNG), Export all (⌘E: logs,
  terminal and every plot), export directory, Settings (⌘,).
- **View menu:** UI scale (⌘+/⌘−), theme, reset layout.
- **Status bar:** connected, waiting for the device or hub approval, hub busy, device not
  responding, or reconnecting.
- **Linux, no access to a device:** a window with the commands that grant it (the `dialout`
  group; `70-uclog.rules`).

## TUI

```sh
uclog tui                                          # connects when exactly one device matches, else opens Connect
uclog tui -d tcp://lab-pc:9000 --files-dir ./files
uclog tui -d serial:/dev/ttyACM0 --plain | grep ERROR   # streaming: ts:level:file:line:text
uclog tui --json --level WARN > warnings.jsonl          # streaming: one JSON record per line
uclog tui --plot-csv plots/                        # port-60 plot data to CSV (the TUI does not draw plots)
```

- **Views:**
  - **1 Logs**, with the terminal below it.
  - **2 Terminal** (port 61).
  - **3 Commands** (port 0).
  - **4 Device**: device info, logdata and counters.
  - **5 FW Update**.
- **Pop-ups:**
  - **F1** Help.
  - **F2** Connect: devices, local and mDNS hubs, BLE scan, or a typed address.
  - **F3** Load logdata.
  - **Ctrl+S** Settings.
- **Logs:**
  - Filters: level (`+`/`-`), source (`f`, or `s` for the selected row's source), line (`n`) and
    regex search (`/`).
  - `p` pauses and `d` discards filtered lines.
  - `c` copies the row (OSC 52 plus the OS clipboard) and Ctrl+E exports CSV.
  - Error records are always shown and are counted in the status bar.
- **Terminal:**
  - Line mode with history (saved in the settings).
  - Character mode with Ctrl+T.
  - ANSI colours are shown.
  - F6–F12 are buttons.
  - Semihost stdout appears in the terminal, and stdin reads take the next line.
- **Commands:** buttons from the settings. A string sends a text line; a map sends CBOR, with
  `$name` placeholders asked for when pressed (as the legacy GUI did). CBOR responses are
  pretty-printed.
- **Leaving the Terminal view:** **Esc**, or **Alt+1–5** from any view (digits are typed there).
  **Ctrl+Q** quits.
- **Reconnect:** a direct device that goes away is reconnected every second until it is back.
- **Settings:**
  - Stored in `settings.json` in the platform config directory (override with `--config`).
  - The logdata token is kept in the OS keyring.
  - On first run, `~/ucLogger.json` is imported: buttons, history, max lines and the files
    directory.

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
    `--allow-origin` (default: `localhost` and `127.0.0.1` pages, VS Code webviews, and the hosted
    web app `https://uclog.unitcircle.ca`).
  - Others are accepted with `--allow-all`, `--allow-from CIDR`, or the saved allow-list.
  - Otherwise the operator is asked on the console: yes, no, or always (saved to `hub.allow` in the
    config directory). Without a console the client is rejected, and a log line says how to allow
    it.
- **Backlog:**
  - On opening the device, the hub asks for its device info (an empty port-62 frame, F7). Direct
    clients do the same on connect.
  - Device frames are kept while no client is connected (`--backlog`, default 1 MiB).
  - A new client gets the device info that applies to its first frame, then the backlog, then one
    pulse that marks the end of the backlog, then live data.
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

Linux builds need `pkg-config`, `libudev-dev` and `libdbus-1-dev`. `packaging/linux/70-uclog.rules` gives users
access to Zephyr USB devices.

On macOS, `build.rs` embeds `packaging/macos/Info.plist` into `uclog` and `Info-hub.plist` into
`uclog-hub` (their signing identifiers, `ca.unitcircle.uclog` and `ca.unitcircle.uclog-hub`).
Their `NSBluetoothAlwaysUsageDescription` lets CoreBluetooth work from the command line.

### Windows

Tested on Windows 11 x64 (2026-10-25): the tests pass and the TUI, GUI and `term` work with
serial, BLE and a remote hub. Building needs rustup and the Visual Studio C++ Build Tools
(`winget install Rustlang.Rustup`).

- BLE throughput depends on the Bluetooth adapter's driver. With Intel's 2022 driver a BLE burst
  ran at ~28 kB/s, with its 2026 driver at ~82 kB/s (firmware/README.md, "BLE throughput with
  Windows 11"). Install the current driver from the adapter's maker; Windows Update's is often
  years old.
- Set the logdata domain and token in the TUI or GUI settings
  ([Logdata service settings](#logdata-service-settings)). `setx` is for command-line use and
  test machines, and reaches only windows opened afterwards.
- The logdata cache is `%USERPROFILE%\.cache\uclog`, as on the other platforms.
- direnv is not recommended on Windows; `.envrc` adds nothing there except `.secrets`. It runs
  `.envrc` with the first `bash` on `PATH`, and STM32CubeCLT's `make\bin\bash` can't run it. With
  Git's bash it runs, but it replaces `Path` with a Unix-style `PATH` (`/c/Users/…`), after which
  that PowerShell can no longer find programs, direnv included. Use the settings or `setx`
  instead.
- The semihost sandbox is not tested against Windows symlinks, junctions and `\\?\` paths yet (plan
  07 task 34).

### WSL2

Tested on WSL2 with Ubuntu 22.04 (2026-10-25). Install the Linux packages above and rustup.

- WSL has no USB or Bluetooth of its own. Run a hub where the device is attached (Windows, or
  another machine) and connect to it by address: `uclog tui -d tcp://192.168.1.20:9000`. WSL's
  network translation blocks mDNS, so `.local` names and hub discovery don't work from WSL.
- There is no keyring: WSL has no D-Bus session bus, so the TUI and GUI skip the keyring and
  can't save a token. Set the domain in the settings, and the token with `--logdata-token` or
  `export UCLOG_LOGDATA_TOKEN=…`.
- The GUI runs through WSLg. Install `adwaita-icon-theme-full` for mouse cursors. On Ubuntu
  22.04 `adwaita-icon-theme` has only the theme's name. Without a theme winit warns on every
  cursor change; `uclog` hides that warning unless `-v` is given.
- If the GUI fails with "Connection reset by peer" (Wayland) or "Failed to open connection to X
  server", WSLg's compositor has crashed: `/mnt/wslg/stderr.log` shows `weston … terminated with
  signal 11`. `wsl --shutdown` in Windows, then reopen Ubuntu.

### Releases

Release builds: `.github/workflows/release.yml` runs on a `v*` tag and builds Linux, macOS and
Windows (x64 and arm64) archives with checksums. Its `macos` job runs
`packaging/macos/release.sh`: it signs `uclog` and `uclog-hub` (hardened runtime) and builds
**ucLog.app**, a universal `uclog` that starts the GUI when opened from the Finder, in
`uclog-vN-macos.dmg`; it notarises and staples them. To check it locally, with the Developer ID
certificate in the keychain and a notarytool profile:

```sh
MACOS_SIGN_IDENTITY="Developer ID Application: Unit Circle Inc. (6CQYR849L3)" \
NOTARY_PROFILE=uclog-notary packaging/macos/release.sh vN ARM64.tar.gz X64.tar.gz OUT_DIR
```

Without `NOTARY_PROFILE` it signs but does not notarise (nothing is sent to Apple). The app's icon
is `packaging/macos/ucLog.icns`, made from `icon.svg` by `icon.sh`. Windows signing and armv7 are
not set up yet.
