# ucLog for VS Code

Logs, a device terminal, commands, plots and FW updates for ucLog devices, in a VS Code panel. It
is the same logger as the ucLog web client, connected through `uclog-hub`, which is bundled.

## Connecting

- **ucLog: Connect…** lists USB devices (STLINK, FTDI and Zephyr CDC-ACM serial ports, HID),
  hubs on this machine and on the network (mDNS), recently used hubs, *Scan for Bluetooth
  devices…*, *Enter hub address…*, and a simulated device.
- A local device is opened by a **private hub** on 127.0.0.1, started for it. Another program can
  then use the device only through that hub. With `uclog.keepHubRunning` (on by default) the hub
  keeps running after you disconnect or close the logger, so logs keep buffering; reopening the
  logger shows them. Connecting to another device stops the kept hub: a board's ST-LINK UART,
  USB CDC and HID are listed as separate devices, but the board serves one at a time and would
  stay on the link the kept hub keeps talking on. Hubs stop when the window closes.
- A **remote hub** is reached over TCP. Its operator may have to accept the connection first
  ("waiting for hub approval"). A hub that already has a client says "hub busy".
- The **Devices** view (ucLog in the activity bar) lists the same devices; click one to connect.
- The status bar shows the device and its state: connected, device not responding, waiting for
  approval, hub busy.

## The logger

- **Logs**: filters for level, source, line and text; per-source colours. Click a `file:line` to
  open the source in the workspace.
- **Terminal** (port 61), **Commands** (port 0), **Plots** (port 60), **Device** (port 62 and the
  logdata in use), **FW update**.
- **ucLog: Open Device Terminal** shows port 61 in a VS Code terminal too
  (`uclog.terminal.mode: pseudoterminal` opens it on connect).

## Logdata

Logdata is chosen by the device's `app_hash`, in this order:

1. files loaded with **ucLog: Load Logdata…** (`.logdata` or an ELF with `.logdata_cbor`);
2. the logdata cache shared with `uclog`, the Python library and the device builds
   (`$UCLOG_CACHE_DIR`, else `$XDG_CACHE_HOME/uclog`, else `~/.cache/uclog`). Downloads and loaded
   files are saved there under their `app_hash`. **ucLog: Clear Logdata Cache** empties it, after
   asking;
3. the logdata service, when `uclog.logdata.domain` is set (the token is set with
   **ucLog: Set Logdata Token** and kept in VS Code's secret storage). If the service refuses the
   token but the account allows reading without one, the logdata is read without it and the
   extension offers to remove the token;
4. the workspace: a `<app_hash>.logdata` file, or the ELF next to a `*.signed.bin` whose hash
   matches.

## FW update

**ucLog: Flash Firmware…** offers the workspace's `*.signed.bin` files, newest first, or any file.
Port 59 is tried first, then port 3.

## Semihost

The device may open and create files under `uclog.semihost.root` (default: `uclog-files` in the
first workspace folder, created when the device first creates a file). Paths that leave that folder
are refused. The default is a subfolder so that a device cannot write `.git/hooks` or `.vscode/`. Its stdout and stderr appear in the terminal.

## Settings

| Setting | |
|---|---|
| `uclog.hubPath` | a `uclog-hub` to use instead of the bundled one |
| `uclog.keepHubRunning` | keep private hubs running after disconnecting (default on) |
| `uclog.baud` | serial baud rate (1000000) |
| `uclog.usbFilters` | USB serial filters `VID:PID` |
| `uclog.logdata.domain`, `uclog.logdata.url` | the logdata service |
| `uclog.semihost.root` | the semihost folder |
| `uclog.maxLogLines` | log lines kept |
| `uclog.commands.buttons`, `uclog.terminal.buttons` | buttons (`{"label", "cmd"}`) |
| `uclog.terminal.onConnect` | lines sent to the terminal after connecting |
| `uclog.terminal.mode` | `webview` or `pseudoterminal` |

## Data

The extension keeps its data in VS Code's storage for it (what the logger remembers: history,
colours, recent hubs) and in its secret storage (the token). **ucLog: Reset All Data** clears both;
uninstalling removes them. The one exception is the shared logdata cache above, which the other
ucLog tools use too: it is kept on reset and uninstall. Nothing is installed outside the
extension's folder.
