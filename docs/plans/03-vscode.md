# Plan 03 — VS Code extension

Shared context, protocol and decisions: [README.md](README.md). D# = decisions (README §1),
Q-x = open questions (README §7). Builds on [02-web.md](02-web.md) and [01-hub-server.md](01-hub-server.md).

## 1. Goal

A VS Code extension that installs and uninstalls cleanly (Marketplace, Open VSX, `.vsix`). It shows
the web UI in a panel, shares its code with the web version, and reaches devices through an
automatically spawned `uclog-hub` (D19).

## 2. Design

VS Code webviews have no Web Serial, WebHID or Web Bluetooth, and native Node modules for USB/BLE
are fragile across VS Code releases. So:

- **Local USB/BLE/UDP device**: the extension spawns a **private hub**:
  `uclog-hub hub -d <uri> --bind 127.0.0.1 --port 0 --no-mdns --json-ready --exit-on-stdin-eof`. It reads
  `{"port":N}` from stdout and connects. Loopback means no approval prompt (D14). The hub's backlog
  means logs captured while the panel was closed are shown on reopen.
  - Setting `uclog.keepHubRunning` (default `true` while the window is open): keep the hub running
    after the panel closes, so logs keep buffering. It is stopped on `deactivate`.
- **Device list**: `uclog-hub list --json` (Serial/HID with filters incl. Zephyr CDC, BLE scan on
  demand, mDNS hubs with `busy` flag).
- **Remote hub**: from mDNS or a manual address. Connects over TCP **from the extension host** (Node
  `net`), so there are no browser Origin rules. The remote hub's operator may need to approve
  (D14), so the UI shows "waiting for hub approval…". A busy hub → "in use".
- **Bundled binaries (D19)**: one universal VSIX contains `uclog-hub` for `linux-x64`,
  `linux-arm64`, `linux-armhf`, `darwin-arm64`, `darwin-x64`, `win32-x64` and `win32-arm64`, under
  `bin/<platform>-<arch>/`. It is chosen at run time from `process.platform`/`process.arch`. Exec
  bits are set on first run (VSIX unzip drops them). macOS binaries are signed and notarised
  (Gatekeeper). `uclog.hubPath` overrides this, for development or a system install.
- **Data path**: ext host TCP socket ⇄ `postMessage` ⇄ webview. In the webview, a `MessagePortLink`
  implements the `Link` interface. The web worker architecture (Plan 02 §3) runs unchanged, with the
  transport worker receiving bytes from the `MessagePortLink`.

## 3. Shared vs VS Code-specific

| Concern | Shared | VS Code-specific |
|---|---|---|
| Protocol, decode, FW update (59→3), semihost logic, workers | `@uclog/core`, `@uclog/workers` | — |
| UI | `@uclog/ui` / `<uc-log>` | theme bridge `--vscode-*` → `--uc-*` |
| Transport | `Link` interface | `MessagePortLink` + `HostBridge` (spawn hub, TCP socket, mDNS via `bonjour-service`) |
| Logdata fetch/cache | resolver | the ext host fetches (no CORS; the `vscode-webview://` origin would be rejected) and caches in `globalStorageUri/logdata/` |
| Token | — | `SecretStorage` |
| File pickers | `pickFile()` hook | `vscode.window.showOpenDialog` |
| Semihost FS | `FsProvider` | ext-host provider sandboxed to `uclog.semihost.root` (default `${workspaceFolder}`); time calls are answered by the hub (D31) |

Webview CSP: `default-src 'none'`, nonce scripts, `worker-src blob:`.

## 4. VS Code features

- Activation on commands/view only (no `*`).
- **Commands**: Open Logger, Connect… (quick pick: devices + hubs + "Enter hub address…"),
  Disconnect, Load Logdata…, Flash Firmware…, Set Logdata Token, Clear Logdata Cache, Reset All Data.
- **Views**: *Devices* tree (kind, serial, busy/connected) + main WebviewPanel
  (`retainContextWhenHidden`).
- **Workspace awareness**: scan `**/build/**/zephyr.signed.bin` and `**/*.logdata`.
  - FW update suggestions.
  - SHA-512 (after the 512-byte header) matched against `app_hash` for automatic local logdata.
- **Terminal mode**: `webview` (xterm.js) or `pseudoterminal` (port 61 in a native VS Code terminal).
- **Click-through**: `file:line` in log rows opens the source.
- **Status bar**: device + state (connected / device not responding / waiting for approval / busy).
- **Settings**: `uclog.hubPath`, `uclog.keepHubRunning`, `uclog.logdata.domain`/`url`,
  `uclog.semihost.root`, `uclog.usbFilters`, `uclog.maxLogLines`, `uclog.commands.buttons`,
  `uclog.terminal.buttons`.

## 5. Clean add/remove

- No extension dependencies, no global installs, no PATH changes. Binaries live inside the
  extension folder.
- State only in `globalStorageUri` + `SecretStorage`. VS Code removes these on uninstall, and
  *Reset All Data* clears them manually.
- Spawned hubs are child processes, killed on `deactivate` and when the extension host exits.
- `extensionKind: ["ui", "workspace"]`, so in Remote-SSH/WSL/containers it runs locally, where the
  USB devices are.

## 6. Work breakdown

| # | Task | Size |
|---|---|---|
| 1 | Scaffold (esbuild, `@vscode/test-electron`), activation, commands | S |
| 2 | `HostBridge`: binary selection + exec bit, spawn/stop, `--json-ready`, `list --json`, TCP client, busy/approval states, mDNS | M |
| 3 | `MessagePortLink`, webview host page, CSP, workers in the webview | S |
| 4 | Theme bridge; file/FS/secret hooks; ext-host logdata fetch + cache | S |
| 5 | Workspace scan → FW suggestions + hash matching | S |
| 6 | Devices tree, status bar, pseudoterminal mode, file:line links | M |
| 7 | Packaging: pull `uclog-hub` release artifacts for all targets into `bin/`, verify checksums, `vsce package`; publish Marketplace + Open VSX + GitHub release | S |

Depends on: Plan 02 tasks 2–7, 9, 10 and Plan 01 (`--port 0`, `--json-ready`, `list --json`).

## 7. Risks

- VSIX size is about 7 targets × 4–6 MB ≈ 30–40 MB. Accepted (D19).
- Some antivirus products flag unsigned exes spawned from extension folders. Sign the Windows binaries.
