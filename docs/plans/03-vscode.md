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
| Logdata fetch/cache | resolver | the ext host fetches (no CORS; the `vscode-webview://` origin would be rejected) and caches in the cache shared with the other hosts, `~/.cache/uclog` (decided 2026-10-25; it was `globalStorageUri/logdata/`, still read) |
| Token | — | `SecretStorage` |
| File pickers | `pickFile()` hook | `vscode.window.showOpenDialog` |
| Semihost FS | `FsProvider` | ext-host provider sandboxed to `uclog.semihost.root` (default `${workspaceFolder}/uclog-files`); time calls are answered by the hub (D31) |

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
  *Reset All Data* clears them manually. The exception is the shared logdata cache
  (`~/.cache/uclog`), which the other hosts and the device builds use too: it is kept.
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

## 8. Implementation notes (Phase 6)

Implemented in [`web/apps/vscode`](../../web/apps/vscode/README.md). Where it differs from the plan
above:

- **Build and versions:**
  - Vite builds both bundles; there is no esbuild dependency. The extension host bundle is
    CommonJS, about 48 KB. The webview is one ES module (139 KB gzip, workers inlined) plus
    uPlot's lazy chunk.
  - The extension needs VS Code 1.100 or later (`@types/vscode` 1.100.0) and was tested with
    1.133 on macOS.
  - There is no `activationEvents` entry: VS Code activates the extension from its commands and
    its view.
- **Devices and hubs:**
  - `uclog-hub list` gained `--hubs SECS` (hubs on this machine from the registry, plus mDNS)
    and `--ble-scan SECS`. The list code moved into `uclog-hub` (`list` module), and
    `uclog list` uses it too. There is no `bonjour-service`; mDNS comes from the hub.
  - Private hubs run as
    `uclog-hub -d URI --bind 127.0.0.1 --port 0 --no-mdns --json-ready --exit-on-stdin-eof`.
    They are in this machine's registry, so `uclog list` shows them as busy local hubs. The
    extension hides its own hubs from its list.
  - The hub keeps retrying a device it cannot open (missing, or held by another program). Its
    `device: …` warnings are shown in the logger as notes.
  - Binaries live in `bin/<process.platform>-<process.arch>/`, so the 32-bit Pi is `linux-arm`
    (not `linux-armhf`). The exec bit is set when it is missing; VS Code 1.133 keeps it.
  - The **simulated device** is offered everywhere, run by a private hub (`-d loopback:`).
- **Data path:**
  - In the webview, an `ExtLink` passes bytes to and from the extension host over `postMessage`.
    In the extension host, a `HubConnection` (Node `net`) talks to the hub.
  - The link opens on the hub's first byte. Busy, approval and the 60 s timeout work as in the
    web's WebSocket link.
  - `MainLink` gained `kind: 'hub'` and `onApproval`. `Client.connect` takes
    `{kind: 'external', link}`, and its `forward` links carry `hub`, so time syscalls are left to
    the hub.
- **Shared code (Plan 02 packages):**
  - `UcLog.host` takes a `UcHost` with hooks for:
    - the Connect UI;
    - file pickers;
    - `file:line` click-through;
    - saving exports;
    - opening settings;
    - a settings backend;
    - host services: logdata cache, fetch, local builds, semihost files.
  - The workers reach those services over MessagePorts (`@uclog/workers` `rpc.ts`). Semihost
    errors keep their errno names across the hops.
  - The resolver gained the local-build step (`sibling`) after the logdata service, as in the
    native resolver.
- **Logdata:**
  - The extension host fetches only from the configured service URL. The token stays in
    SecretStorage and never enters the webview.
  - The cache is the one shared with the other hosts and the device builds
    (`$UCLOG_CACHE_DIR`, else `$XDG_CACHE_HOME/uclog`, else `~/.cache/uclog`):
    `<app_hash>.logdata`. The older `globalStorageUri/logdata/` is still read, and *Clear Logdata
    Cache* empties both after asking.
  - The workspace is searched for `**/<app_hash>.logdata`, then for the ELF next to a
    `**/*.signed.bin` with a matching hash. Image hashes are remembered while a file's mtime and
    size are unchanged.
- **Settings:**
  - Configuration lives in VS Code settings; the extension added `uclog.baud` and
    `uclog.terminal.onConnect`.
  - What the logger remembers lives in `globalState`: history, colours, recent hubs and layout.
  - The logger's *Settings…* opens VS Code's settings filtered to ucLog.
- **Semihost files** are served under `uclog.semihost.root`, `${workspaceFolder}/uclog-files` by default (a subfolder, so that a device cannot write `.git/hooks` or `.vscode/`; created on the first create).
  Paths are checked again in the extension host, including symlinks that lead outside the root
  (EACCES).
- **Terminal mode:** the pseudoterminal works in line mode and the device echoes, as in the
  TUI and GUI. *Open Device Terminal* shows it; `uclog.terminal.mode: pseudoterminal` opens it
  on connect.
- **Click-through** opens the source in the editor group beside the logger. A partial path
  picks the workspace file whose path ends with it; several matches give a quick pick.
- **`keepHubRunning`:** reopening the logger reconnects to a hub kept running, so its backlog
  appears. When the setting is off, the hub stops when the link closes.
- **Extra commands:** Open Device Terminal, Show Hub Output, Refresh, Scan for Bluetooth Devices,
  Enter Hub Address….
- **Tests:**
  - Plain `@vscode/test-electron` is not used. Instead, Playwright's Electron driver runs a real
    VS Code and drives the UI, including the webview.
  - **Vitest:**
    - hub binary selection and the exec bit, `list --json`, private hub start, stop and errors;
    - TCP link states: busy, approval, timeout, refused;
    - semihost files, including errno names and sandbox escapes;
    - cache, workspace lookup, fetch;
    - the RPC between webview and workers.
  - **Playwright in VS Code (9):**
    - the simulated device with logdata found in the workspace;
    - click-through;
    - semihost console output and time through the hub;
    - the pseudoterminal;
    - hub lifetime with `keepHubRunning` off and on (the backlog appears on reopen);
    - the Devices view;
    - Flash Firmware with a workspace image;
    - a hub by address, busy and then connected.
  - The same suite passes against an installed VSIX (`UCLOG_VSIX=…`).
  - **Hardware** (`e2e/hw.spec.ts`): the NUCLEO-L452RE-P through a private hub, with logdata
    from the workspace. `test` and `burst 5000` decoded 5353 logs with no error records.
  - Semihost file operations through the extension are tested in parts (the extension host's
    files, and the RPC). The simulated device makes no file calls.
- **Packaging:**
  - `scripts/package.mjs` fills `bin/` from this machine's build (`--local`), or from the
    release archives with their checksums verified (`--from DIR`). It then runs `vsce` 3.9.2.
  - A VSIX with one target is 2.3 MB. Each target adds about 6 MB.
  - `release.yml` has a `vsix` job, and `web.yml` runs the VS Code tests under xvfb.
- **Not done yet** (moved to [Plan 07](07-release.md)):
  - Marketplace and Open VSX publishing.
  - The armv7 hub in the VSIX.
  - Signed hubs.
  - Runs on Windows and Linux.
  - Remote-SSH/WSL.
  - HID and BLE devices through the extension on hardware.
