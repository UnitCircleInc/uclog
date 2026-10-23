# ucLog client v2

Next-generation ucLog clients for Unit Circle devices. Every client shares one wire protocol:

| Component | Directory | Plan | Status |
|---|---|---|---|
| Protocol spec + test vectors | [`spec/`](spec/protocol.md) | [README §3](docs/plans/README.md) | Phase 0 done (firmware changes pending) |
| Shared Rust crates + `uclog list/decode/term/fw-update` | [`native/`](native/README.md) | [README §4, §6](docs/plans/README.md) | **Phase 1**: done, hardware verification pending |
| Hub (Serial/HID/BLE/UDP ↔ TCP/WebSocket/UDP) | `native/` (Rust) | [01](docs/plans/01-hub-server.md) | planned |
| Web version / `<uc-log>` component | `web/` (TypeScript) | [02](docs/plans/02-web.md) | planned |
| VS Code extension | `web/apps/vscode` | [03](docs/plans/03-vscode.md) | planned |
| Native GUI (`uclog gui`) | `native/` (Rust) | [04](docs/plans/04-native-gui.md) | planned |
| Native TUI (`uclog tui`) | `native/` (Rust) | [05](docs/plans/05-tui.md) | planned |
| Python library for scripts | `python/` | [06](docs/plans/06-python-client.md) | planned |

Start with [docs/plans/README.md](docs/plans/README.md): decisions, architecture and phasing.

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
