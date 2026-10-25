# ucLog client v2

Next-generation ucLog clients for Unit Circle devices. Every client shares one wire protocol:

| Component | Directory | Plan | Status |
|---|---|---|---|
| Protocol spec + test vectors | [`spec/`](spec/protocol.md) | [README §3](docs/plans/README.md) | Phase 0 done (firmware changes pending) |
| Shared Rust crates + `uclog list/decode/term/fw-update` | [`native/`](native/README.md) | [README §4, §6](docs/plans/README.md) | **Phase 1**: done; verified on hardware over serial (HID, BLE, UDP and FW update: Phase 7) |
| Bare-metal device library + test firmware (NUCLEO-L452RE-P: UART; NUCLEO-C071RB: UART, USB CDC and HID; STM32WBA55G-DK1: UART and BLE) | [`firmware/`](firmware/README.md) | — | test targets for the hosts |
| Hub (Serial/HID/BLE/UDP ↔ TCP/WebSocket/UDP) | [`native/`](native/README.md) (`uclog hub`, `uclog-hub`) | [01](docs/plans/01-hub-server.md) | **Phase 2**: done; verified on hardware over serial |
| Web version / `<uc-log>` component | [`web/`](web/README.md) (TypeScript) | [02](docs/plans/02-web.md) | **Phase 5**: done; verified in Chrome (Playwright) with the simulated device and the native hub, and on hardware through the hub; Web Serial/HID/BLE with real devices and npm publishing: Phase 7 |
| VS Code extension | [`web/apps/vscode`](web/apps/vscode/README.md) | [03](docs/plans/03-vscode.md) | **Phase 6**: done; verified in VS Code (Playwright) with the simulated device and hubs, and on hardware through a private hub; Marketplace publishing and other OSes: Phase 7 |
| Native GUI (`uclog gui`) | [`native/`](native/README.md) | [04](docs/plans/04-native-gui.md) | **Phase 4**: done; verified with the loopback device (headless UI tests) and on hardware (macOS); packaging and other platforms: Phase 7 |
| Native TUI (`uclog tui`) | [`native/`](native/README.md) | [05](docs/plans/05-tui.md) | **Phase 3**: done; verified on hardware (macOS) |
| Python library for scripts | [`python/`](python/README.md) | [06](docs/plans/06-python-client.md) | **Phase 2b**: done; verified with the loopback device and on hardware (macOS); PyPI publishing: Phase 7 |
| Release: CI, signing, PyPI, platforms, hardware coverage | — | [07](docs/plans/07-release.md) | **Phase 7**: planned |

Start with [docs/plans/README.md](docs/plans/README.md): decisions, architecture and phasing.

Building the Rust workspace on macOS needs Homebrew's `rustup` and direnv (the repository's
`.envrc` puts `cargo` on `PATH`): see [native/README.md](native/README.md#setup).

## Protocol

- [`spec/protocol.md`](spec/protocol.md) — normative wire protocol v1
- [`spec/vectors/`](spec/vectors/README.md) — test vectors every implementation must pass
- [`spec/reference/`](spec/reference/ucref.py) — executable reference (Python standard library only)
- [`spec/firmware-changes.md`](spec/firmware-changes.md) — what the firmware must change or confirm

```sh
python3 spec/reference/gen_vectors.py --check   # vectors are up to date
python3 spec/reference/check_vectors.py         # vectors pass against the reference
```

The legacy Python client (`uclog-client`) is frozen. It stays in its own repository until the
v2 components reach parity.
