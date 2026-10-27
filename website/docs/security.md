# Security

What can reach a device through ucLog, and what a device can reach on your machine.

## The hub

A [hub](hub.md) gives its one client full control of the device: logs, the terminal, commands and
firmware updates. So it decides who that client may be:

- **Programs on this machine** are let in.
- **Web pages** on this machine are let in only from the origins on `--allow-origin`: by default
  `http(s)://localhost` and `127.0.0.1` pages, VS Code's webviews, and the hosted web app
  (`https://uclog.unitcircle.ca`). Any other page that tries to open the hub (a WebSocket carries
  the page's origin) is treated as a stranger, so a website you happen to visit cannot drive your
  device.
- **Other machines** are let in with `--allow-all`, a matching `--allow-from CIDR`, or a host saved
  in `hub.allow`. Otherwise the hub holds the connection and asks on its console:
  *yes*, *no*, or *always* (saved in `hub.allow`, in the [settings folder](cli.md#settings)). With
  no console, it turns them away and logs how to allow them. An allowed machine is trusted as a
  whole, including any web page in a browser on it.
- **One client at a time**; the others are told the hub is busy.

The hub listens on every IPv4 interface (`--bind 127.0.0.1` keeps it to this machine) and announces
itself over mDNS (`--no-mdns` stops that). Hubs that the Python library and VS Code start for
themselves listen on 127.0.0.1 only and are not announced.

**On the network**, use TLS: `--tls-cert` and `--tls-key` add TLS and `wss://` on the same port.
Clients check the certificate: `uclog` against the public certificate authorities it knows, a
browser against its own list, so use a certificate from a public CA for the hub's name (not a
self-signed one). Without TLS, logs and commands cross the network in the clear.

## Web pages and your local network

Chrome asks before a public site may reach this machine or the local network (*Local network
access*). The hosted web app needs that permission to use a hub on your machine; a page that does
not get it cannot even open the connection, and the app says so. The one-file web app, opened
from disk, has the origin `null`: a hub lets it in only when started with
`--allow-origin null`.

Web Serial, WebHID and Web Bluetooth always ask you to choose the device, and a page only sees
the devices you gave it.

## Files on the host (semihost)

A device can use files and the console on the host it is connected to, through semihosting. Every
host keeps that in a box:

- **No commands, no deleting or renaming.** A device cannot run a program on the host, and cannot
  remove, rename or link files: those calls are not supported.
- **One folder.** Files are opened and created only inside the folder the host chose, and paths
  that leave it (`..`, absolute paths, links that point outside, names that are not portable) are
  refused. With no folder chosen, file calls fail.

| Host | The folder |
|---|---|
| `uclog gui`, `uclog tui`, `uclog term` | `--files-dir`, or the one in the settings; none by default |
| The hub | `--files-dir`, only with `--syscalls`; otherwise the connected client's |
| VS Code | `uclog.semihost.root`: `uclog-files` in the first workspace folder by default, so a device cannot write `.git/` or `.vscode/` |
| Web app | the folder you pick (*Serve files from folder…*), with the browser's permission |
| Python | `files_dir=`; none by default |

The device's stdout and stdin only reach the terminal view and the script's `console()` and
`stdin=`.

## Logdata tokens

A token for a logdata service is a secret, and the hosts keep it like one:

| Host | Kept in |
|---|---|
| `uclog` | the OS keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service), never the settings file |
| VS Code | its secret storage |
| Web app | the browser session only, unless you choose to remember it (then the browser's storage for uclog.unitcircle.ca) |
| `<uc-log>` | the `logdataToken` property, set by your page: never in the HTML |
| Scripts and CI | `UCLOG_LOGDATA_TOKEN`, from your CI's secrets |

A host sends the token only to the logdata service, with the app hash it asks for; logs never
leave your machine. On Windows, `setx UCLOG_LOGDATA_TOKEN …` stores the token in the registry in
plain text: on a desktop, use the settings (the keyring) instead.

## Firmware updates

Any client of a device can update its firmware. The device decides what it accepts: the update
protocol carries signed images (`*.signed.bin`), and checking the signature is the bootloader's
job, not the host's. Limit who can connect to a device (above) as you would limit who can flash
it.
