# Python

The `uclog` package is for tests and scripts: read decoded logs, send to the terminal and command
ports, answer the device's file calls. It talks to devices through a hub, and starts one for you
(the wheel includes `uclog-hub`).

```sh
pip install uclog
```

```python
import uclog

with uclog.connect(logdata="build/app.elf") as dev:
    print(dev.device_info)                    # board, app_hash, ...
    dev.send(61, "help\r")                    # str or bytes, to any port
    print(dev.recv(61, timeout=1.0))          # the next payload on port 61, or None
    rsp = dev.command({"cmd": "get-info"})    # port 0: a CBOR request, and its reply
    for rec in dev.logs(duration=5.0):        # decoded logs for 5 s
        print(rec)                            # 1.234:INFO:src/main.c:10:Hello World!
```

## Which device

`uclog.connect(host=None, device=None, ...)`:

1. With `host` (or `$UCLOG_HOST`): that hub, such as `"lab-pc.local:9001"`.
2. Otherwise **a hub already running** on this machine (or, with `discover_remote=True`, on the
   network); with `device=` (a URI or a serial number), that device's hub.
3. Otherwise it **starts a hub** for the one local device, or the one `device=` names. The hub
   listens on 127.0.0.1 only and stops when the connection closes, or when the script dies.

It never guesses: several matches raise `MultipleHubsError`, with the candidates. A hub with a
client already raises `HubBusyError`; one waiting for its operator's approval raises
`ApprovalTimeoutError` after 60 s. `connect()` returns once the device has sent something
(`DeviceNotRespondingError` after 10 s).

## The connection

| | |
|---|---|
| `logs(timeout=, duration=)` | decoded `LogRecord`s: `ts`, `level`, `file`, `line`, `text`, and `error` for records that could not be decoded |
| `send(port, data)`, `recv(port, timeout=)` | any port; at most 1443 bytes per send |
| `command(obj, timeout=2.0)` | a CBOR request on port 0, and the reply whose `rsp` matches its `cmd` |
| `console(timeout=)` | the device's stdout and stderr (semihost) |
| `device_info` | the device's port-62 information |
| `frames()` | every payload, undecoded |

On connect, a hub first replays what the device sent while no one was connected;
`skip_backlog=True` drops that. `uclog.aio.connect()` is the asyncio version.

## Logdata and files

`logdata=` takes files (an ELF or `.logdata`); then the shared cache, then the logdata service
with `logdata_domain=` and `logdata_token=` (or `UCLOG_LOGDATA_DOMAIN`, `UCLOG_LOGDATA_TOKEN`).
See [Logdata](logdata.md).

The script answers the device's semihost calls: stdout and stderr go to `console()`, stdin reads
call `stdin=`, and file calls work inside `files_dir=` only (without it they fail with `EACCES`).

## Examples

[`python/examples/`](https://github.com/UnitCircleInc/uclog/tree/main/python/examples):
`capture_logs.py` (logs to the console), `terminal.py` (an interactive terminal, Windows too) and
`command.py` (a request and its reply).
