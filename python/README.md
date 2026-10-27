# uclog: Python library for scripts

Plan: [docs/plans/06-python-client.md](../docs/plans/06-python-client.md).

A small library for test scripts and automation. It does four things:

- finds a running ucLog hub, or starts one;
- connects to it;
- sends and receives on any port;
- decodes logs with CBOR logdata.

All device I/O goes through the native hub (`uclog-hub`), so the library is pure Python and
behaves the same on every OS. It needs Python 3.9 or later and has no dependencies. The optional
`uclog[mdns]` extra adds `zeroconf`, for finding hubs on other machines.

```sh
pip install uclog            # platform wheels bundle uclog-hub
pip install "uclog[mdns]"    # also find remote hubs over mDNS
```

The `py3-none-any` wheel has no `uclog-hub`. It uses `$UCLOG_HUB`, or `uclog-hub` (or `uclog`) on
`PATH`.

## Quick start

```python
import uclog

with uclog.connect(logdata="build/zephyr/zephyr.elf") as dev:
    print(dev.device_info)                    # port-62 map: board, app_hash, ...
    dev.send(61, "version s\r")               # str or bytes, to any port
    print(dev.recv(61, timeout=1.0))          # next payload on port 61, or None
    rsp = dev.command({"cmd": "get-info"})    # port 0: CBOR request, waits for rsp == cmd
    for rec in dev.logs(duration=5.0):        # decoded log records for 5 s
        print(rec)                            # 1.234:INFO:src/main.c:10:Hello World!
```

The examples in [examples/](examples/) replace the legacy scripts:

| Example | |
|---|---|
| `capture_logs.py` | logs, filtered by level, source and text (replaces `uclog-cli.py`) |
| `terminal.py` | a line-mode terminal, also piped (replaces `logcmd.py`; works on Windows too) |
| `command.py` | a CBOR request on port 0 and its reply (replaces `StreamClient.txrx`) |
| `decode_capture.py` | decode a raw capture of a device's bytes offline |
| `pytest/` | a pytest fixture for hardware tests (`--uclog-device`), and tests that use it |

`tests/test_examples.py` runs every example against the simulated device. The API reference is
generated from the docstrings for the docs site (`website/build.py`).

## Choosing a hub

`uclog.connect(host=None, device=None, ...)` goes through these steps in order:

1. **`host` given** (or `$UCLOG_HOST`): connect to it. Examples: `"192.168.1.20"`,
   `"labpc.local:9001"`, `"tcp://[::1]:9000"`. The default port is 9000.
2. **One running hub** on this machine (registry), or also on the network with
   `discover_remote=True`: connect to it. With `device=` (a URI like `serial:/dev/ttyACM0` or a
   serial number), only that device's hub counts.
3. **Several hubs** match: `MultipleHubsError`. Its `.candidates` lists them; pass `host=`. The
   library never guesses.
4. **No hub**: it runs `uclog-hub list --json`. If exactly one device is found, or `device=` names
   one, it starts a hub for it and connects. If no device matches it raises `NoHubError`; if
   several match, `MultipleHubsError`.

A hub the library started is **stopped when the connection closes**, at the end of the `with`
block, or when the script exits. If the script is killed, the hub sees its stdin close and exits.
Hubs that were already running are left alone. A started hub listens on `127.0.0.1` only and is
not advertised over mDNS (`hub_args=["--bind", "0.0.0.0"]` changes that); local clients still
find it in the registry. Scripts starting a hub for the same device at the same time take turns,
so only one hub is started.

`connect()` returns once **the device** has sent something (not just the hub's backlog replay),
waiting up to `device_timeout` (10 s; a hub opens a BLE device in about 5 s). Otherwise it raises
`DeviceNotRespondingError`.

```python
uclog.list_hubs()                   # [HubInfo(name, host, port, device, serial_number, busy, pid, ...)]
uclog.list_hubs(discover_remote=True)
uclog.list_devices()                # [DeviceInfo(kind, uri, name, serial, description)]
hub = uclog.start_hub("serial:/dev/ttyACM0")   # keep a hub across several connections
uclog.stop_hub(hub.pid)             # or uclog.stop_hub("localhost:9001"); only a registered hub
```

A hub serves **one client at a time**. While the GUI, the TUI or VS Code is connected,
`connect()` raises `HubBusyError`. A remote hub may hold a new connection open, without data, while
its operator approves it. `connect()` waits up to `approval_timeout` (60 s), then raises
`ApprovalTimeoutError`.

## Connection

| Call | |
|---|---|
| `send(port, data)` | bytes, or str as UTF-8; at most 1443 bytes |
| `send_frame(payload)` | a whole payload, frame header included |
| `recv(port, timeout=None)` | next payload on `port` (bytes), or `None` after `timeout` |
| `command(obj, timeout=2.0)` | CBOR map on port 0; returns the reply whose `rsp` equals `obj["cmd"]`, or `None`. Other port-0 frames stay queued for `recv(0)` |
| `logs(timeout=None, duration=None)` | iterator of `LogRecord`s, ending after `timeout` seconds without one or `duration` seconds in all |
| `frames(timeout=None, duration=None)` | every payload, raw |
| `console(timeout=None)` | semihost stdout/stderr: `(fid, bytes)` |
| `device_info` | the port-62 map; waits up to 2 s for it, or until `connect()`'s `device_timeout` ends if that is later |
| `alive` | bytes arrived within the last 2 s |
| `dropped` | entries dropped from full queues, by port, `"logs"`, `"frames"` or `"console"` |
| `notes` | logdata choices and problems (also logged to the `uclog` logger) |
| `load_logdata(path)` | use another logdata file from now on |
| `close()` | sends what is still queued (up to 2 s), then closes; also stops a hub the library started |

`LogRecord(ts, level, target, file, line, text, error)`:
- `ts` is seconds since the connection opened.
- `error` is set for error records: `no_logdata`, `unknown_format`, `missing_arg`, `extra_bytes`,
  `framing_invalid`, and so on, with the same texts as the native tools.
- `str(rec)` gives the legacy `ts:level:file:line:text` line.
- Logs from the backlog are decoded with the logdata of the device info that preceded them, so
  logs from before a reflash use the old image's logdata. An image with no logdata gives
  `no_logdata` records.

**Queues** are bounded by `max_queue` (10 000). When one is full, the oldest entry is dropped and
counted in `dropped`.

**Backlog:** on connect, the hub first replays what the device sent while no client was
connected. `skip_backlog=True` drops that replay.

**asyncio:** `uclog.aio.connect(...)` takes the same arguments. Its connection has the same
calls, awaited, except that `device_info` becomes the method `await dev.device_info(timeout)`.
`logs()` and `frames()` are async iterators.

```python
async with await uclog.aio.connect(device="serial:/dev/ttyACM0") as dev:
    await dev.send(61, "help\r")
    print(await dev.recv(61, timeout=1))
```

## Logdata

`connect(logdata=...)` takes one file or a list: `.logdata`, or an ELF file with a
`.logdata_cbor` section. Each device info (port 62) selects the logdata for its `app_hash`, in this
order:

1. **The given files.** A file named `<app_hash>.logdata` is used only for that hash.
2. **The cache directory.** `cache_dir`, which defaults to the native tools' cache
   (`$UCLOG_CACHE_DIR`, else `$XDG_CACHE_HOME/uclog`, else `~/.cache/uclog`), holding
   `<app_hash>.logdata` files. The firmware builds copy theirs there.
3. **fw.unitcircle.ca.** Used when `logdata_domain` (or `$UCLOG_LOGDATA_DOMAIN`) is set, with
   `logdata_token` / `$UCLOG_LOGDATA_TOKEN`. The download is saved to the cache. A refused token
   (401) is retried once without it, for accounts that allow reading without a token; the
   resolver's `token_refused` is then set and `notes` says so.
4. **The first given file**, with a warning in `notes`.

`uclog.LogData.load(path).decode(addr, args)` decodes without a connection.

## Port 63 (semihost)

A connected script answers the device's syscalls. The hub answers the time calls itself.

- **stdout and stderr** writes go to `console()`.
- **stdin reads** call `stdin=` (a function taking the maximum byte count and returning bytes or
  str), or get EOF.
- **File operations** (OPEN, MKDIR) get `EACCES`, unless `files_dir=DIR` is given. Then they
  work inside `DIR` only: absolute paths, `..` escapes, symlinks that leave it and names that are
  not portable (protocol.md §5.5.5) fail with `EACCES`. Errors use newlib errno values.

The connection's own thread answers syscalls, so a script that never reads from it still does not
stall the device.

## Migrating from the legacy package

The legacy repository's `uclog` package uses the same import name. Uninstall it, or use a separate
virtual environment.

| Legacy | Now |
|---|---|
| `uclog.StreamClient(target, host, raw, stream=0, enc='utf8')` | `uclog.connect(host=..., device=...)` |
| `sc.tx(obj)` with `enc='cbor'` | `dev.send(port, uclog.cbor.dumps(obj))`, or `dev.command(obj)` for request/reply |
| `sc.tx(text)` with `enc='utf8'` | `dev.send(port, text)` |
| `sc.rx(timeout)` | `dev.recv(port, timeout)`; `uclog.cbor.loads(...)` for CBOR |
| `sc.txrx(obj)` | `dev.command(obj)` |
| `enc='cbor,crc,cobs'` (legacy port-3 FW update) | `uclog fw-update` |
| `--target /dev/tty...` (opens the port directly) | `device="serial:/dev/tty..."` (through a hub) |
| `uclog-cli.py` | `examples/capture_logs.py`, or `uclog tui --plain` |
| `logcmd.py` | `examples/terminal.py`, or `uclog term` |

FW update from Python is out of scope for v1 (decision 30). Use `uclog fw-update`.

## Development

```sh
cd python
uv run --no-project --with pytest pytest          # or: pip install -e ".[test]" && pytest
```

The integration tests use `$UCLOG_HUB` or `native/target/{release,debug}/uclog-hub`, and the
simulated `loopback:` device. Without either executable they are skipped.

To build a **platform wheel** that bundles the hub:

```sh
UCLOG_HUB_BINARY=../native/target/release/uclog-hub UCLOG_WHEEL_PLAT=macosx_11_0_arm64 python -m build --wheel
```
