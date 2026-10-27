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
| 2 | Repository secrets for signing (§3): Apple Developer ID certificate + notarisation API key, Windows code-signing certificate. **Apple ready 2026-10-26**: Developer ID Application certificate (team 6CQYR849L3) exported as .p12 and an App Store Connect API key (Developer role), both in 1Password; the key is also the `uclog-notary` notarytool profile on the dev Mac. Windows: Azure Artifact Signing still to set up (identity validation takes days). Secrets go into GitHub once the repository exists (task 1) | 01 §9.10 | S |
| 3 | A dry run of `release.yml` (`workflow_dispatch`): every target builds, checksums, draft GitHub release, wheels artifact, VSIX artifact. Check that the `macos-13` (Intel), `ubuntu-22.04-arm` and `windows-11-arm` runners are available to this repository (the ARM ones may not be for a private repository); otherwise build macOS x64 on `macos-14` (cross) and Linux arm64 with `cross`. Until there is a remote: `.github/act-release.sh` runs the Linux and macOS rows locally with act (no Windows, no release job) | 01 §9.10, 03 §8 | S |

## 3. Packaging and signing

| # | Task | From | Size |
|---|---|---|---|
| 4 | macOS: codesign (hardened runtime) and notarise `uclog` and `uclog-hub`, arm64 and x64. Check that the embedded `Info.plist` still lets BLE work after signing. **Local test 2026-10-26 (arm64)**: `codesign --force --options runtime --timestamp`, no entitlements, then `ditto -c -k` and `notarytool submit --wait` were Accepted with no issues. A quarantined copy is "Notarized Developer ID" to Gatekeeper, and `uclog list --ble-scan 5` found the WBA55. A bare binary can't be stapled, so Gatekeeper checks online. Both binaries get the identifier `ca.unitcircle.uclog` from the shared Info.plist; give `uclog-hub` its own (`--identifier` or a second plist) **Done 2026-10-27 (CI not run yet: no repository)**: `release.yml`'s `macos` job runs `packaging/macos/release.sh` on the two macOS archives: codesign (hardened runtime, timestamp) of both binaries in each, then one notarisation of them with the app; the repacked archives replace the unsigned ones (artifacts `unsigned-macos-*` → `dist-macos`). `uclog-hub` now has its own `Info-hub.plist` (`ca.unitcircle.uclog-hub`). Secrets: `MACOS_CERTIFICATE_P12` (base64), `MACOS_CERTIFICATE_PASSWORD`, `APPLE_API_KEY_P8`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`; without them the files go on unsigned. The release job passes the job's `signed` output to the manifest (`UCLOG_SIGNED_MACOS`). Checked locally with the `uclog-notary` profile: both submissions Accepted, the app and the `.dmg` stapled, `spctl`: Notarized Developer ID | 01 §9.10 | M |
| 5 | macOS GUI: a `ucLog.app` bundle in a `.dmg`, signed, notarised and stapled, alongside the plain binary **Done 2026-10-27 (CI not run yet: no repository)**: `uclog-vN-macos.dmg` holds `ucLog.app`, a universal `uclog` (`lipo`) with `ucLog.icns` (from `icon.svg`, `icon.sh`); `uclog` starts the GUI when it runs with no arguments from an app bundle. The app is notarised with the binaries and stapled, then the `.dmg` is signed, notarised and stapled. The download page offers it to Macs. Checked locally: the app from the signed `.dmg` opens the GUI. Started from the Finder its working directory is `/`, and the GUI's search for `*.signed.bin` (4 levels) read the user's folders, so macOS asked for Music, Documents and the rest: the search now skips the home directory and above, and the GUI and TUI use the home directory when started at `/` | 04 §6 | M |
| 6 | Windows: sign `uclog.exe` and `uclog-hub.exe` (x64, arm64). Optional MSI | 01 §9.10, 04 §6 | M |
| 7 | Linux: an AppImage for the GUI; the tarball keeps `70-uclog.rules` | 04 §6 | M |
| 8 | Linux armv7 build (`cross`) in `release.yml`, for the Pi, the armv7 wheel and the VSIX (`linux-armv7` archive → `bin/linux-arm`) **Done 2026-10-27 (CI not run yet)**: not with `cross`, whose 0.2.5 images are Ubuntu 16.04 (glibc 2.23, gcc 5): the `linux-armv7` row runs on `ubuntu-22.04` with Ubuntu's `gcc-arm-linux-gnueabihf` and the armhf `libudev`/`libdbus` from ports.ubuntu.com (glibc 2.35, as the other Linux builds: Raspberry Pi OS bookworm and later). The wheels job adds `manylinux_2_35_armv7l`; the VSIX takes it as `linux-arm`; the download page lists it (and picks it for a 32-bit Arm browser). Checked locally in an Ubuntu 22.04 container: `uclog` 14.9 MB, `uclog-hub` 3.8 MB; on Debian bookworm armv7l (emulated) both run, `uclog term` with the simulated device | 01 §9.10, 03 §8 | S |
| 9 | First-run hint when serial or hidraw access is denied on Linux: show how to install the udev rules (GUI dialog, TUI/CLI message) **Done 2026-10-27**: `LinkError::access_hint()` (uclog-link): on Linux, a serial or HID open refused with *Permission denied* gives the commands (the `dialout` group; `70-uclog.rules`). The CLI prints them under the error, the GUI shows them in a window (with Copy buttons) instead of reopening the Connect dialog, the TUI puts them in the Device tab, and the hub logs them once. Unit test of the detection; not yet seen on a Linux machine | 04 §6 | S |
| 10 | Python wheels and the VSIX take the **signed** `uclog-hub` from the release build **Done 2026-10-27**: the wheels and vsix jobs take the `dist-*` artifacts, so the macOS hub in them is the signed one (and the Windows one, once task 6 signs it in the same way) | 06 §1, 03 §2 | S |

## 4. Publishing

| # | Task | From | Size |
|---|---|---|---|
| 11 | PyPI: create the `uclog` project and set up a **trusted publisher** (GitHub OIDC) for this repository, `release.yml` and a `pypi` environment with required reviewers. Do the same on TestPyPI | 06 §9 | S |
| 12 | A publish job in `release.yml` (`pypa/gh-action-pypi-publish`, `id-token: write`, no API token). It publishes the wheels and sdist on a `v*` tag, after the build jobs. Publish to TestPyPI first, then install from it on 3 OS **Done 2026-10-27 (not run yet)**: `release.yml` jobs `testpypi` → `testpypi-install` (Linux, macOS, Windows: `pip install` from TestPyPI, the wheel's own `uclog-hub` against the simulated device) → `pypi`, by trusted publishing in the environments `testpypi` and `pypi`; only in `UnitCircleInc/uclog`. Setup: `docs/plans/08-go-public.md` | 06 §9 | S |
| 13 | Versioning: SoloVer (plan 08 §2.2a). One host version line `vN`, starting at `v4` (the device repository already has `v1`–`v3`), mapped to each manifest (Rust workspace, Python package, `package.json` files, VSIX; all `0.1.0` now); CI derives them from the tag and checks them. **Done 2026-10-26** as plan 08 task 1: `scripts/version.py set N` before tagging; `release.yml` checks the tag | 08 | S |
| 13a | npm: publish `@uclog/element` (Changesets, provenance) and a versioned single-file build on GitHub Releases or a CDN **Done 2026-10-27 (not run yet)**: `release.yml` job `npm` (environment `npm`): `pnpm pack`, then `npm publish --provenance` (`next` for pre-releases), with `NPM_TOKEN` for the first publish and trusted publishing after. The package: `@uclog/ui` and `@uclog/workers` are bundled, so devDependencies; repository, homepage, keywords and a README. The single-file build is on each release and the site (`/element/N/`). Without Changesets: the version comes from `scripts/version.py`. `cem` no longer writes its output path into `web/package.json` (`packagejson: false`) | 02 §7, §10 | S |
| 13b | VS Code: create the `unitcircle` publisher (Marketplace) and namespace (Open VSX); a publish job in `release.yml` with `vsce publish` and `ovsx publish` (tokens as secrets, `--require-all` targets). Check the repository URL in the manifest **Done 2026-10-27 (not run yet)**: `release.yml` job `vscode` (environment `vscode`): the release's VSIX with `vsce publish` and `ovsx publish` (`VSCE_PAT`, `OVSX_PAT`); not for pre-releases. The manifest's repository was `unitcircle/uclog-clientv2`: now `UnitCircleInc/uclog`, with homepage and bugs | 03 §6 | S |
| 14 | First release: tag, publish the draft GitHub release, release notes | — | S |

## 5. Platforms

| # | Task | From | Size |
|---|---|---|---|
| 15 | GUI on Linux (X11 and Wayland; file dialogs through xdg-desktop-portal), Windows x64/arm64, and Pi 4/5 (glow on GLES 3.0) **Done for now 2026-10-26**: covered in part by the Windows 11 and WSL2 (Ubuntu) runs of 2026-10-25/26 (native build and tests, TUI, GUI, hub, BLE, the VS Code extension and Chrome with the three boards); a full retest follows the next round of feature changes | 04 §8, §9 | M |
| 16 | TUI manual matrix: Windows Terminal, conhost, macOS Terminal/iTerm2, Linux, over SSH **Done for now 2026-10-26**: covered in part by the Windows 11 and WSL2 (Ubuntu) runs of 2026-10-25/26 (native build and tests, TUI, GUI, hub, BLE, the VS Code extension and Chrome with the three boards); a full retest follows the next round of feature changes | 05 §8 | S |
| 17 | Python library on Linux and Windows (CI, then one hardware run each), including the hub exiting when the script dies on Windows **Done for now 2026-10-26**: covered in part by the Windows 11 and WSL2 (Ubuntu) runs of 2026-10-25/26 (native build and tests, TUI, GUI, hub, BLE, the VS Code extension and Chrome with the three boards); a full retest follows the next round of feature changes | 06 §9 | S |
| 18 | Hub on Linux and Windows: `flock` on the serial device, the registry and runtime directories, and the console approval prompt **Done for now 2026-10-26**: covered in part by the Windows 11 and WSL2 (Ubuntu) runs of 2026-10-25/26 (native build and tests, TUI, GUI, hub, BLE, the VS Code extension and Chrome with the three boards); a full retest follows the next round of feature changes | 01 §3, §7 | S |

## 6. Devices and hardware

| # | Task | From | Size |
|---|---|---|---|
| 19 | HID devices on Windows and Linux, including the Windows interrupt-read check (issue 7). Add `--hid-poll` (`get_input_report` fallback) only if that check fails; the C071 test firmware answers `GET_REPORT` with an empty report, so it needs a real fallback first. ✅ macOS: the NUCLEO-C071RB test firmware's HID, through the hub, directly and in VS Code **Done for now 2026-10-26**: covered in part by the Windows 11 and WSL2 (Ubuntu) runs of 2026-10-25/26 (native build and tests, TUI, GUI, hub, BLE, the VS Code extension and Chrome with the three boards); a full retest follows the next round of feature changes | 01 §3, §10 | M |
| 20 | BLE devices (btleplug, maximum chunk size) on macOS, Linux and Windows. ✅ macOS: the STM32WBA55G-DK1 test firmware, through the hub, directly and in VS Code (~72 kB/s: macOS uses a 15 ms interval and limits each connection event's length). Measure throughput with Linux and Windows centrals **Done for now 2026-10-26**: covered in part by the Windows 11 and WSL2 (Ubuntu) runs of 2026-10-25/26 (native build and tests, TUI, GUI, hub, BLE, the VS Code extension and Chrome with the three boards); a full retest follows the next round of feature changes | 01 §10 | M |
| 21 | UDP devices | 01 §10 | S |
| 22 | Serial adapters other than the STLINK VCP: FTDI, and CDC-ACM on Windows and Linux. ✅ macOS: CDC-ACM (`2FE3:0100`, the NUCLEO-C071RB test firmware) | 01 §10 | S |
| 23 | FW update (ports 59 and 3) on hardware, including cancel and the port-3 fallback | Phase 1 | S |
| 24 | Unplug/replug during a session; device UART power-down and wake (no pulses for over 2 s, then traffic) | 01 §10 | S |
| 25 | mDNS hub discovery from a second machine (`uclog list`, Connect dialogs, Python) | 06 §9 | S |
| 26 | Hub soak: 1 h at the maximum log rate with an intermittently stalled client; memory stays bounded by `--backlog` | 01 §10 | S |
| 27 | GUI and TUI against each device kind and through a hub (the manual matrix of 04 §7 task 12) **Done for now 2026-10-26**: covered in part by the Windows 11 and WSL2 (Ubuntu) runs of 2026-10-25/26 (native build and tests, TUI, GUI, hub, BLE, the VS Code extension and Chrome with the three boards); a full retest follows the next round of feature changes | 04 §7 | M |
| 27a | Web: Web Serial, WebHID and Web Bluetooth with real devices (the Serial and HID choosers cannot be automated: Chrome's DevTools `DeviceAccess` covers only Bluetooth, and seeded permissions are ignored); Chrome and Edge on Windows, macOS and Linux. The NUCLEO-C071RB test firmware offers UART, CDC and HID at once. ✅ Web Bluetooth, Chrome on macOS: `web/e2e/ble.spec.ts` with the STM32WBA55G-DK1 test firmware **Done for now 2026-10-26**: covered in part by the Windows 11 and WSL2 (Ubuntu) runs of 2026-10-25/26 (native build and tests, TUI, GUI, hub, BLE, the VS Code extension and Chrome with the three boards); a full retest follows the next round of feature changes | 02 §10 | M |
| 27b | VS Code: the extension on Windows and Linux (private hubs, serial, HID, BLE scan), over Remote-SSH and WSL (`extensionKind` ui: USB stays local), and semihost file calls from a real device. *Load Logdata…* and *Browse…* use native dialogs, so they are checked by hand **Done for now 2026-10-26**: covered in part by the Windows 11 and WSL2 (Ubuntu) runs of 2026-10-25/26 (native build and tests, TUI, GUI, hub, BLE, the VS Code extension and Chrome with the three boards); a full retest follows the next round of feature changes | 03 §8 | M |

## 7. Remaining features

| # | Task | From | Size |
|---|---|---|---|
| 28 | `uclog hub --config FILE` (in the plan's CLI, not implemented) | 01 §8 | S |
| 29 | GUI terminal character mode (each key sent at once), like the TUI's | 04 §9 | S |
| 30 | GUI fonts: bundle a fallback font for the symbols and emoji egui's default fonts lack, which now show as boxes. Watch the size target (≤ 15 MB) | 04 §9 | S |

## 7a. Left over from the code review (2026-10-25)

The review's other findings are fixed. These were left:

| # | Task | From | Size |
|---|---|---|---|
| 31 | Web folder FS (File System Access API, `transports-web/src/fs.ts`): writes go to the browser's swap file and are lost if the page closes before the device closes the file; an `O_RDWR` handle's reads don't see its own writes. Needs a different storage design (for example write-through on each write, or reads from the pending writable) **Done 2026-10-26**: a file open for writing is held in memory, shared by its handles, and written to the file 200 ms after a write (`FLUSH_MS`) and on close; O_APPEND writes go to the end. Unit tests with a fake folder that keeps writes in a swap file like Chrome (they fail on the old code) and `web/e2e/fs.spec.ts` in Chrome on the origin-private file system | review | M |
| 32 | NUCLEO-C071RB USB (`board/usb.c`): CLEAR_FEATURE(HALT) and SET_INTERFACE don't reset the endpoint's data toggle, and SET_FEATURE(HALT) doesn't stall. Not seen on macOS; check with a host that clears a halt (Linux libusb, Windows) and fix **Done 2026-10-26**: halt now stalls and is reported by `GET_STATUS`; clearing it and `SET_INTERFACE` reset the toggle. Tested with libusb on macOS (the CDC data interface can be claimed): before the fix, the first command after clearing an OUT halt was lost every time (6/6), and after `SET_INTERFACE` 3 times in 6; after it, no loss in 14 OUT-halt and 14 `SET_INTERFACE` runs during bursts, nor in 16 IN-halt runs reading one packet per transfer (a larger read that ends in a stall is an error, and libusb drops the packets it held). Not tested with Windows or Linux drivers clearing a halt themselves | review | S |
| 33 | Web: remove duplicated code: `FsError`/`OpenFlags`/errno names (core and vscode), `LinkState`, the approval/busy state machine (`WebSocketLink` and `HubConnection`), `bytes()`/`toBytes()`, `DEFAULT_LOGDATA_URL`, `portableName` (core and vscode `files.ts`) **Done 2026-10-26**: the extension imports `FsError`, `OpenFlags`, `ERRNO`, `portableName` and `DEFAULT_LOGDATA_URL` from `@uclog/core` (its host bundle inlines it); `LinkState` and the admission state machine are core's `HubAdmission` (`hub.ts`), used by `WebSocketLink` and `HubConnection`, so each state is reported once; one `toBytes()` in the extension's `messages.ts` | review | S |
| 34 | Windows: build and test the semihost sandbox (native `uclog-semihost`: `O_NOFOLLOW` is `cfg(unix)`; check that a symlink or junction in the root cannot lead out of it on Windows) **Done 2026-10-25**, on Windows 11 for native, Python and the VS Code extension. No link or `..` variant leads out. Device names (`CON`, `NUL`, `COM1`), `:` streams and names ending in a dot or space were accepted; they are now refused on every host (protocol.md §5.5.5). Native opens with `FILE_FLAG_OPEN_REPARSE_POINT` on Windows; Python opens the resolved path with `O_NOFOLLOW` | review | S |
| 35 | Rerun the hardware e2e the review didn't run after its fixes: the VS Code extension with the three test boards, `web/e2e/hw.spec.ts` and `web/e2e/ble.spec.ts`; BLE scanning (stops as soon as the device is found) on Linux and Windows **Done 2026-10-26** (macOS): `hw.spec.ts` and the VS Code `hw` test pass on the L452 (ST-LINK UART) and the C071 (ST-LINK UART, CDC, HID), and `ble.spec.ts`, `hw.spec.ts` (ST-LINK UART) and the VS Code BLE test on the WBA55. Its ST-LINK-V3 and the C071's ST-LINK both report the serial number 0047002E3133511335333335, so with both plugged in STM32CubeProgrammer lists only one (firmware/README.md). The VS Code test had not been rerun since the semihost root became `uclog-files` and the logdata cache became `~/.cache/uclog` (now a cache per test). Windows: the hub opens the WBA55 1.2–1.4 s after starting (the scan stops at the first match; its limit is 4 s). Linux not checked: WSL has no Bluetooth | review, 27a, 27b | S |
| 36 | Optional: NUCLEO-C071RB CDC throughput (~350 kB/s, CPU bound; see firmware/README.md "Why HID and CDC are slower than the bus"): word copies straight from the log ring into the packet memory (measured +10–16%), a double-buffered bulk IN endpoint, `log_tx()` without all interrupts off, a faster COBS encoder **Done 2026-10-26**: 332 → 464.5 kB/s (`burst 20000`). Word copies into the packet memory; a partial packet waits for a full one or the next 1 ms tick (the interrupt fired once per ~19-byte log, 21% of the CPU; now 6%); a faster COBS encoder and inline argument stores in `log.c`. Not done: double buffering and encoding with interrupts on, since the CPU making logs is the limit, not USB (firmware/README.md) | review | S |

## 8. Not in this phase

- The firmware implementation of F1–F7 in the product firmware (Phase 0). It lives outside this
  repository; the test firmware in `firmware/` already has F6 and F7.
