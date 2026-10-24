# Plan 07 — Release, platforms and hardware coverage

Shared context, protocol and decisions: [README.md](README.md). D# = decisions (README §1).

## 1. Goal

Phases 1–6 and 2b are built and verified on macOS, with the NUCLEO-L452RE-P over serial. This
phase collects what is still missing before a first public release:

- signed packages and published artefacts;
- runs on Linux, Windows and the Pi;
- the other device links on real hardware;
- the few features the earlier phases left out.

These items came from the "Not done yet" notes of plans [01](01-hub-server.md),
[02](02-web.md), [03](03-vscode.md), [04](04-native-gui.md), [05](05-tui.md) and
[06](06-python-client.md), and from `.github/workflows/release.yml`. The VSIX (Plan 03) bundles
the hub binaries this phase signs.

## 2. Repository and CI

| # | Task | From | Size |
|---|---|---|---|
| 1 | Create the GitHub repository and push. Check that `native.yml`, `python.yml`, `spec.yml` and `web.yml` pass (the native and Python ones on Linux, Windows and macOS; `web.yml` includes the VS Code tests under xvfb); they have never run (no remote yet) | 06 §9, 03 §8 | S |
| 2 | Repository secrets for signing (§3): Apple Developer ID certificate + notarisation API key, Windows code-signing certificate | 01 §9.10 | S |
| 3 | A dry run of `release.yml` (`workflow_dispatch`): every target builds, checksums, draft GitHub release, wheels artifact, VSIX artifact | 01 §9.10, 03 §8 | S |

## 3. Packaging and signing

| # | Task | From | Size |
|---|---|---|---|
| 4 | macOS: codesign (hardened runtime) and notarise `uclog` and `uclog-hub`, arm64 and x64. Check that the embedded `Info.plist` still lets BLE work after signing | 01 §9.10 | M |
| 5 | macOS GUI: a `ucLog.app` bundle in a `.dmg`, signed, notarised and stapled, alongside the plain binary | 04 §6 | M |
| 6 | Windows: sign `uclog.exe` and `uclog-hub.exe` (x64, arm64). Optional MSI | 01 §9.10, 04 §6 | M |
| 7 | Linux: an AppImage for the GUI; the tarball keeps `70-uclog.rules` | 04 §6 | M |
| 8 | Linux armv7 build (`cross`) in `release.yml`, for the Pi, the armv7 wheel and the VSIX (`linux-armv7` archive → `bin/linux-arm`) | 01 §9.10, 03 §8 | S |
| 9 | First-run hint when serial or hidraw access is denied on Linux: show how to install the udev rules (GUI dialog, TUI/CLI message) | 04 §6 | S |
| 10 | Python wheels and the VSIX take the **signed** `uclog-hub` from the release build | 06 §1, 03 §2 | S |

## 4. Publishing

| # | Task | From | Size |
|---|---|---|---|
| 11 | PyPI: create the `uclog` project and set up a **trusted publisher** (GitHub OIDC) for this repository, `release.yml` and a `pypi` environment with required reviewers. Do the same on TestPyPI | 06 §9 | S |
| 12 | A publish job in `release.yml` (`pypa/gh-action-pypi-publish`, `id-token: write`, no API token). It publishes the wheels and sdist on a `v*` tag, after the build jobs. Publish to TestPyPI first, then install from it on 3 OS | 06 §9 | S |
| 13 | Versioning: one version for the Rust workspace and the Python package (both `0.1.0` now); the tag must match, checked in CI | — | S |
| 13a | npm: publish `@uclog/element` (Changesets, provenance) and a versioned single-file build on GitHub Releases or a CDN | 02 §7, §10 | S |
| 13b | VS Code: create the `unitcircle` publisher (Marketplace) and namespace (Open VSX); a publish job in `release.yml` with `vsce publish` and `ovsx publish` (tokens as secrets, `--require-all` targets). Check the repository URL in the manifest | 03 §6 | S |
| 14 | First release: tag, publish the draft GitHub release, release notes | — | S |

## 5. Platforms

| # | Task | From | Size |
|---|---|---|---|
| 15 | GUI on Linux (X11 and Wayland; file dialogs through xdg-desktop-portal), Windows x64/arm64, and Pi 4/5 (glow on GLES 3.0) | 04 §8, §9 | M |
| 16 | TUI manual matrix: Windows Terminal, conhost, macOS Terminal/iTerm2, Linux, over SSH | 05 §8 | S |
| 17 | Python library on Linux and Windows (CI, then one hardware run each), including the hub exiting when the script dies on Windows | 06 §9 | S |
| 18 | Hub on Linux and Windows: `flock` on the serial device, the registry and runtime directories, and the console approval prompt | 01 §3, §7 | S |

## 6. Devices and hardware

| # | Task | From | Size |
|---|---|---|---|
| 19 | HID devices on Windows and Linux, including the Windows interrupt-read check (issue 7). Add `--hid-poll` (`get_input_report` fallback) only if that check fails; the C071 test firmware answers `GET_REPORT` with an empty report, so it needs a real fallback first. ✅ macOS: the NUCLEO-C071RB test firmware's HID, through the hub, directly and in VS Code | 01 §3, §10 | M |
| 20 | BLE devices (btleplug, maximum chunk size) on macOS, Linux and Windows | 01 §10 | M |
| 21 | UDP devices | 01 §10 | S |
| 22 | Serial adapters other than the STLINK VCP: FTDI, and CDC-ACM on Windows and Linux. ✅ macOS: CDC-ACM (`2FE3:0100`, the NUCLEO-C071RB test firmware) | 01 §10 | S |
| 23 | FW update (ports 59 and 3) on hardware, including cancel and the port-3 fallback | Phase 1 | S |
| 24 | Unplug/replug during a session; device UART power-down and wake (no pulses for over 2 s, then traffic) | 01 §10 | S |
| 25 | mDNS hub discovery from a second machine (`uclog list`, Connect dialogs, Python) | 06 §9 | S |
| 26 | Hub soak: 1 h at the maximum log rate with an intermittently stalled client; memory stays bounded by `--backlog` | 01 §10 | S |
| 27 | GUI and TUI against each device kind and through a hub (the manual matrix of 04 §7 task 12) | 04 §7 | M |
| 27a | Web: Web Serial, WebHID and Web Bluetooth with real devices (the choosers cannot be automated: Chrome's DevTools `DeviceAccess` covers only Bluetooth, and seeded permissions are ignored); Chrome and Edge on Windows, macOS and Linux. The NUCLEO-C071RB test firmware offers UART, CDC and HID at once | 02 §10 | M |
| 27b | VS Code: the extension on Windows and Linux (private hubs, serial, HID, BLE scan), over Remote-SSH and WSL (`extensionKind` ui: USB stays local), and semihost file calls from a real device. *Load Logdata…* and *Browse…* use native dialogs, so they are checked by hand | 03 §8 | M |

## 7. Remaining features

| # | Task | From | Size |
|---|---|---|---|
| 28 | `uclog hub --config FILE` (in the plan's CLI, not implemented) | 01 §8 | S |
| 29 | GUI terminal character mode (each key sent at once), like the TUI's | 04 §9 | S |
| 30 | GUI fonts: bundle a fallback font for the symbols and emoji egui's default fonts lack, which now show as boxes. Watch the size target (≤ 15 MB) | 04 §9 | S |

## 8. Not in this phase

- The firmware implementation of F1–F7 in the product firmware (Phase 0). It lives outside this
  repository; the test firmware in `firmware/` already has F6 and F7.
