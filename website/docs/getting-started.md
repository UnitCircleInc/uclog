# Getting started

ucLog is compact binary logging for embedded devices. The device sends each log as the address of
its format string and the raw arguments; the host turns that back into text with the strings it
reads from the firmware's ELF. The same link carries a terminal, commands, plots, files on the host
and firmware updates. ([From device code to screen](device/logging.md) shows how.)

## 1. Pick a host

All of them read the same devices and the same logdata; use the one that fits where you work
([the hosts](hosts.md)).

| Host | Install |
|---|---|
| [Web app](web-app.md) | Nothing: open [uclog.unitcircle.ca/app/](https://uclog.unitcircle.ca/app/) in Chrome or Edge. It also works offline, and as one downloadable file |
| [VS Code](vscode.md) | The **ucLog** extension (Unit Circle) from the extensions view, or the `.vsix` from a [release](https://github.com/UnitCircleInc/uclog/releases) (*Install from VSIX…*). It brings its own `uclog-hub` |
| [`uclog`](cli.md): GUI, TUI and command line | The archive for your platform from the [download page](https://uclog.unitcircle.ca/download/) (or a [GitHub release](https://github.com/UnitCircleInc/uclog/releases)): `uclog` and [`uclog-hub`](hub.md) |
| [Python](python.md) | `pip install uclog` (the wheel includes `uclog-hub`) |

The release archives are `uclog-vN-<platform>.tar.gz` (Linux, macOS) or `.zip` (Windows), for
x64 and arm64, each with a `.sha256`. Put the two programs on your `PATH`.

- **Linux:** serial ports need your user in `dialout` (Debian, Ubuntu) or `uucp` (Arch). For USB
  CDC and HID devices (vendor `2fe3`), install the archive's `70-uclog.rules` as its first lines
  say, then replug.
- **Windows:** the programs are not code-signed yet, so SmartScreen may stop the first run: choose
  *More info*, then *Run anyway*. Check the archive against its `.sha256` first.
- **macOS:** Bluetooth needs the permission macOS asks for on the first scan (System Settings →
  Privacy & Security → Bluetooth, for the terminal app you run `uclog` in).

## 2. Try the simulated device

Every host has a simulated device that logs, answers on the terminal (type `help`) and draws
plots, with no hardware:

- **Web app** and **VS Code**: *Connect…*, then *Simulated device*.
- **Command line**: `uclog tui -d loopback:` (its logs need the sample logdata,
  [`spec/vectors/sample.logdata`](https://github.com/UnitCircleInc/uclog/blob/main/spec/vectors/sample.logdata):
  `-l sample.logdata`).

## 3. Connect a board

1. **Add the device library** to the firmware ([Adding ucLog to a device](device/index.md)), or
   start from one of its [examples](device/index.md#the-examples). The firmware build puts the
   logdata in the hosts' cache.
2. **Plug the board in** and choose it in *Connect…* (or `uclog tui`, which connects when exactly
   one device is found). The hosts list ST-LINK, FTDI and Zephyr (vendor `2fe3`) USB serial ports,
   Zephyr USB HID devices, and Bluetooth LE devices with the ucLog service; other serial adapters
   are added with `--usb-filter VID:PID` or by path. The serial speed is 1 Mbaud unless you set
   another.
3. **The logs appear.** The host finds the logdata by the image's app hash: in an ELF or
   `.logdata` file you load, in its cache, or from a logdata service
   ([Logdata](logdata.md)). Logs it cannot decode are shown as errors with their raw bytes, never
   dropped.

To use the board from another machine, or from several programs in turn, run a
[hub](hub.md) next to it: `uclog hub -d auto`.
