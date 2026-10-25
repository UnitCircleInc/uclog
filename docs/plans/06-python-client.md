# Plan 06 — Python client library (`uclog` package for scripts)

Shared context, protocol and decisions: [README.md](README.md). D# = decisions (README §1),
Q-x = open questions (README §7). Builds on [01-hub-server.md](01-hub-server.md).

## 1. Goal

A small Python library (D24) for test scripts and automation. It replaces `uclog.StreamClient`,
`logcmd.py` and ad-hoc use of the current package. It must:

1. **Start a hub if none is running** for the device.
2. **Select a hub** when more than one is running. The user supplies the address (IP/host[:port]).
3. **Connect, send and receive** data on any port.
4. **Load CBOR logdata** (`.logdata`, or ELF `.logdata_cbor`) and **decode logs**.

It never opens USB/BLE itself; all device I/O goes through the native hub (`uclog-hub`). This keeps
the library pure Python and identical on every OS.

## 2. Packaging

- PyPI package `uclog`, Python ≥ 3.9. **No dependencies** (the CBOR codec is the stdlib-only one
  from `spec/reference/ucref.py`; see §9). Optional extra `uclog[mdns]` adds `zeroconf` for
  finding remote hubs.
- **Platform wheels bundle `uclog-hub`** (like `ruff`): `manylinux` x64/arm64/armv7, macOS
  arm64/x64, Windows x64/arm64, built from the same release artefacts as Plan 03. A pure-Python
  `py3-none-any` wheel is also published for other platforms. It uses `uclog-hub` (or `uclog`) from
  `PATH` or `$UCLOG_HUB`.
- Lives in `python/` in this repository. The legacy code stays in the old `uclog-client` repository (README §4).

## 3. API

```python
import uclog

# Find or start a hub, connect, and attach a decoder.
with uclog.connect(logdata="build/zephyr/zephyr.elf") as dev:
    print(dev.device_info)                 # port-62 map (board, app_hash, …), waits up to 2 s
    dev.send(61, b"version s\r")           # raw bytes to a port
    print(dev.recv(61, timeout=1.0))       # next payload on port 61 (bytes) or None
    rsp = dev.command({"cmd": "get-info"})  # port 0: CBOR request, waits for rsp == cmd
    for rec in dev.logs(timeout=5.0):      # decoded log records until 5 s of silence
        print(rec)                         # LogRecord(ts, level, target, file, line, text, error)

# Explicit choices
uclog.connect(host="192.168.1.20")          # a specific hub (port 9000 by default)
uclog.connect(host="labpc.local:9001")
uclog.connect(device="serial:/dev/ttyACM0") # the hub for that device (start one if needed)
uclog.list_hubs()                           # local registry + (with [mdns]) mDNS; busy flag
uclog.list_devices()                        # `uclog-hub list --json`
```

| Object | Contents |
|---|---|
| `Connection` | `send(port, data)`, `send_frame(raw_payload)`, `recv(port, timeout)`, `frames(timeout)` (all frames, raw), `logs(timeout)`, `command(obj, timeout)`, `device_info`, `alive` (pulses seen within 2 s), `close()` |
| `LogData` | `LogData.load(path)` (`.logdata` or ELF), `decode(frame) → LogRecord`; several files for multi-target |
| `Resolver` | optional: cache dir + `fw.unitcircle.ca` lookup by `app_hash` (README §5.3), using `urllib` (no extra dependency). Enabled when `logdata_domain=` or `UCLOG_LOGDATA_DOMAIN` is set |
| Errors | `NoHubError`, `MultipleHubsError(candidates)`, `HubBusyError`, `ApprovalTimeoutError`, `DeviceNotRespondingError` |

Also usable from async code: `uclog.aio.connect(...)` with the same API (asyncio streams).
The sync API is a thin wrapper around a background I/O thread.

## 4. Hub selection and start-up

`uclog.connect(host=None, device=None, …)`:

1. `host` given → connect to it. Done.
2. Collect running hubs:
   - local registry files (hub plan §7.1), with dead PIDs ignored;
   - mDNS `_uclog._tcp` for 1 s, only with the `[mdns]` extra and `discover_remote=True`.

   Filter by `device` (URI or serial number) if given.
3. **Exactly one** → connect.
4. **More than one** → raise `MultipleHubsError`, which lists name, address, device, serial and
   busy for each. The message tells the user to pass `host=` (or set `UCLOG_HOST`). The library
   never guesses.
5. **None** → `uclog-hub list --json`:
   - exactly one device (or `device` given) → spawn
     `uclog-hub hub -d <uri> --json-ready --exit-on-stdin-eof [--files-dir DIR]` as a child
     process with a pipe on stdin, wait for the JSON line, then connect;
   - zero or several devices → raise `NoHubError` / `MultipleHubsError` with the device list.
6. A hub that the library started is **stopped when the script exits** (D30): on `close()`, at
   the end of the `with` block, or from an `atexit` handler. Hubs that were already running are never
   stopped.
   `uclog.stop_hub(host_or_pid)` is available too.

Spawned hubs bind to the default port (with the 9001…9019 fallback, hub plan §8) on `127.0.0.1`,
without mDNS (`--bind 127.0.0.1 --no-mdns` unless `hub_args` says otherwise): a script's hub is
private, but local tools still find it through the registry. Loopback connections need no approval. Remote hubs may hold the connection for
operator approval; `connect()` waits up to `approval_timeout` (60 s) and then raises
`ApprovalTimeoutError`.

## 5. Protocol handling

- COBS framing, pulses and the 2-byte peek, the same as the other clients (README §3). Vectors from
  `spec/vectors/` run in CI.
- A background thread sends a pulse every 0.5 s and sends the pulse/≥ 50 ms wake gap before the first
  frame after idle (README §3.1). It also tracks liveness (`alive`).
- **Port 63**: connecting means the script answers syscalls (D3). The hub answers the time calls
  itself (D31), so the library only sees file ops. It replies `ENOSYS` to them by default. With
  `files_dir=DIR`, it also implements the sandboxed file ops (newlib errno, D25). This means a
  script that never reads from the connection still does not stall the device.
- Receive queues per port are bounded (`max_queue`, default 10 000 frames). On overflow the oldest
  frame is dropped and `Connection.dropped[port]` is incremented. The backlog replay on connect (hub
  plan §5.1) arrives first; `connect(skip_backlog=True)` discards frames that arrive before the
  first live device pulse.
- Decode errors produce `LogRecord`s with `error` set, using the same texts as README §5.6.

## 6. Logdata and decoding

- Start from `spec/reference/ucref.py` (`LogData`, `decode_log`, `format_c`), which is already
  stdlib-only and matches the vectors. It was derived from the legacy `uclog/logdata.py`, with the
  objdump/DWARF path removed (D11). Add a minimal ELF32 section reader for `.logdata_cbor`, about
  60 lines and no `pyelftools`.
- Decoder switching: each port-62 frame (including the one replayed before the backlog) selects the
  logdata by `app_hash`. The order is: files given to `connect(logdata=…)` whose hash matches, then
  the resolver, then the first given file as a fallback with a warning.

## 7. Work breakdown

| # | Task | Size |
|---|---|---|
| 1 | Package skeleton, CI (pytest on 3 OS × Python 3.9/3.13), vectors runner | S |
| 2 | COBS, scanner, peek, framing; `Connection` I/O thread, pulses, liveness, per-port queues | S |
| 3 | Hub discovery (registry, optional mDNS), spawn + `--json-ready`, errors | S |
| 4 | Port 63: ENOSYS by default; optional sandboxed file ops | S |
| 5 | Logdata port + ELF reader + decoder switching on port 62; optional resolver | S |
| 6 | `command()` (port 0 CBOR), `aio` wrapper | S |
| 7 | Platform wheels with bundled `uclog-hub` (cibuildwheel-style job pulling release artefacts); PyPI publish | S |
| 8 | Docs + migration notes for `StreamClient` / `logcmd.py` users; examples | S |
| 9 | Integration tests against a real `uclog-hub` with a fake device (`-d loopback:` test link from hub plan §10) | M |

Depends on: Plan 01 (registry, `--json-ready`, `list --json`, default-port fallback).

## 8. Risks

- **Single-client hubs** (D3): a script cannot attach while a GUI or VS Code is connected to the
  same hub. It gets `HubBusyError`. Document the workflow: close the GUI, or use a raw
  `--client-syscalls off` session for the script only.
- A script that is killed (SIGKILL, a crashed interpreter, a closed terminal) cannot run its exit
  handler. `--exit-on-stdin-eof` covers this: the OS closes the stdin pipe when the script's process
  dies, and the hub exits. This works the same on Windows, macOS and Linux.
- While a script owns its hub, other clients cannot connect to that device (single client).
- Platform wheel size is about 5 MB each. Accepted.

## 9. Implementation notes (Phase 2b)

Implemented in [`python/`](../../python/README.md). Where it differs from the plan above:

- **API and packaging:**
  - **No `cbor2`**: the package is standard library only. `uclog.cbor` is the reference codec,
    checked by the vectors.
  - **`uclog-hub list [--json]`** was added to the hub. The wheels bundle only `uclog-hub`, and it
    had no way to list devices. With `uclog` on `PATH` instead, the library runs
    `uclog list --json --mdns 0`.
  - **`connect()` wait:** after connecting, it waits for the first byte.
    - Local hub: up to `device_timeout` (5 s), then `DeviceNotRespondingError`. `None` skips the
      wait.
    - Remote hub: up to `approval_timeout`, then `ApprovalTimeoutError`.
    - The hub closes the connection with no data: `HubBusyError`.
  - **Added:**
    - `HubStartError` and `ConnectionClosedError`.
    - `console()` for semihost stdout/stderr. Writes not yet read are joined, so unbuffered stderr
      does not arrive a byte at a time.
    - A `stdin=` callback for the device's stdin reads (EOF without one).
    - `logs()` and `frames()` take `duration=`: with a chatty device, "until `timeout` s of
      silence" may never come.
  - **`aio`** wraps the threaded connection: waits run in the default executor. It does not use
    asyncio streams. `device_info` is `await dev.device_info()`.
- **Hub changes:**
  - **`skip_backlog`**: the hub now sends **one pulse after the backlog replay** (protocol.md §8).
    "The first live device pulse" did not work, because under load the device and the hub never
    idle long enough to pulse. The library finds the marker by counting runs of `00` bytes, so it
    still works when TCP splits a write.
  - **Hub fixes found by the integration tests:**
    - Without a console, `--exit-on-stdin-eof` exited at once. Dropping the question channel ended
      the task that watched stdin.
    - The hub printed `--json-ready` before writing its registry entry and subscribing to `busy`,
      so a client connecting at once left the registry showing `busy: false`.
    - When the script was killed, stderr closed too. tracing-subscriber reported the failed write
      with `eprintln!`, which panicked; with `panic = "abort"`, the hub died without removing its
      registry entry. `log_internal_errors(false)` in `uclog` and `uclog-hub` fixes it.
- **Tests:**
  - spec/vectors, unit tests (CBOR, printf, ELF, the port-63 sandbox), a scripted fake hub (busy,
    silent device, `command()`, wake gap and pulses, queue overflow, console), and integration
    against `uclog-hub` with the `loopback:` device.
  - 50 tests on Python 3.9, 3.13 and 3.14 (macOS), including a script killed with SIGKILL: its
    hub exits and leaves no registry entry.
  - On the NUCLEO-L452RE-P: hub started by serial number, device info, logdata from the cache,
    terminal, semihost selftest (printf, stderr, files 32/1438/4000 bytes), stdin callback, the
    1443-byte frame, hub stopped on close.
- **Not done yet** (moved to [Plan 07](07-release.md)):
  - PyPI publishing (trusted publisher, then a publish job). The release workflow builds the
    wheels as an artifact.
  - CI on Linux and Windows has not run (no remote yet).
  - mDNS discovery is untested without a second machine.
