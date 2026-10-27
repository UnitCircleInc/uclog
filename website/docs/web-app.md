# Web app

[uclog.unitcircle.ca/app/](https://uclog.unitcircle.ca/app/) runs in Chrome and Edge (Web Serial,
WebHID and Web Bluetooth are Chromium-only). It connects to:

- a device on this computer: a serial port (UART, USB CDC), USB HID, or Bluetooth LE;
- a `uclog hub` on this computer or on the network (`ws://` or `wss://`);
- the simulated device, to try it out.

## Offline

After one visit the app also loads with no network: a service worker keeps a copy. When a new
version has been published, the app says so, and the next load uses it.

## One file

`uclog-app-N.html` is the whole app in a single file, for computers with no network or a locked-down
browser. Download it from the app (*Download for offline use*), or from the release on GitHub, and
open it from disk. From a file:

- Web Serial, WebHID and Web Bluetooth work as they do from the site.
- A hub refuses the page unless it was started with `uclog hub --allow-origin null` (a page opened
  from a file has the origin `null`). The app says so when the hub turns it away.
- Logdata comes from the ELF, `.logdata` files or the cache; the logdata service refuses requests
  from a file (origin `null`).

## A hub on this computer, from the site

Chrome asks before a public site such as uclog.unitcircle.ca may connect to this computer or the
local network (*Local network access*). Allow it, and a hub on this computer accepts the site
without asking: `https://uclog.unitcircle.ca` is on a hub's default `--allow-origin` list. If the
permission was denied, the connection never opens; allow it again in the site's settings (the
icon left of the address).
