# The hub

`uclog hub` (also the separate program `uclog-hub`) opens one device and serves it on the network:
to the GUI, TUI, VS Code, the web app or Python scripts, on this machine or anywhere
else. While no client is connected it keeps what the device sends, so nothing is lost between
sessions.

```sh
uclog hub -d auto                                   # the one local USB device, on port 9000
uclog hub -d serial:/dev/ttyACM0 --allow-from 192.168.1.0/24
uclog hub -d auto --tls-cert hub.pem --tls-key hub.key   # also TLS and wss://
uclog hub -d auto --syscalls --files-dir ./files     # the hub answers the device's file calls itself
```

Then connect to `tcp://HOST:9000` (or `ws://`, `wss://`) from any host, or pick the hub in
*Connect…*: hubs are announced over mDNS (`_uclog._tcp`) and found on this machine without it.

## How it behaves

- **One client at a time.** Another is turned away and shows *hub busy*.
- **Port 9000**, or the next free one up to 9019 when `--port` is not given. Raw TCP and
  WebSocket share the port, and TLS too with `--tls-cert` and `--tls-key`.
- **The backlog.** While no client is connected the hub keeps the device's frames (`--backlog`,
  1 MiB by default, oldest dropped first). A client that connects gets the device's information,
  then the backlog, then live data; logs from before a reflash are decoded with the old image's
  logdata. A client that falls behind skips frames (counted), or is disconnected with
  `--slow-client disconnect`.
- **Who may connect.** Programs on this machine, except web pages from origins not on
  `--allow-origin`; others with `--allow-all`, `--allow-from CIDR`, or once the operator says yes
  on the hub's console. See [Security](security.md#the-hub).
- **Files and time (semihost).** The hub answers the device's clock and time calls itself, always.
  Other calls (stdout, stdin, files) go to the connected client, which answers in its own sandbox;
  with no client they fail (`ENOSYS`). With `--syscalls`, the hub answers them all, with files in
  `--files-dir`. If a client leaves while the device waits for an answer, the device gets `EIO`.

## Running one for a lab or CI

- Give it the clients' addresses (`--allow-from 10.0.0.0/8`) or `--allow-all` on a trusted
  network: a hub without a console turns away clients it would otherwise ask about, and logs how
  to allow them.
- `--no-mdns` stops the announcements; `--bind 127.0.0.1` keeps it to this machine.
- `uclog-hub list --json` lists the local USB devices, for scripts that start hubs themselves (the
  Python library does).
- A hub started for tests: `--port 0 --json-ready` prints the port it got as JSON, and
  `--exit-on-stdin-eof` stops it when its parent goes away.

`uclog hub --help` lists every option.
