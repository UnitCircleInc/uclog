# Embedding in a web page

The web app is one element, `<uc-log>`, and any page can use it: a product's support page, a lab
dashboard, an internal tool. It brings the same views (logs, terminal, commands, plots, device
information, firmware update) and connects the same way: Web Serial, WebHID, Web Bluetooth, or a
hub.

## Add it to a page

```html
<script src="https://uclog.unitcircle.ca/element/latest/uclog-element.iife.js"></script>

<uc-log logdata-domain="acme.example" style="height: 600px"></uc-log>
```

That is all: the script defines `<uc-log>` (and the global `UcLog`), and the element shows a
**Connect…** button. It fills its container: give it a height (it is at least 360 px tall).

Which script:

| URL | |
|---|---|
| `/element/N/uclog-element.iife.js` | release N; it never changes. Use this on a page you publish |
| `/element/latest/uclog-element.iife.js` | the newest release |
| `/element/dev/uclog-element.iife.js` | built from `main`; for trying changes |

The script is about 160 KB compressed, with everything it needs inside, the two workers included.
You can also copy it to your own site: it is attached to each
[release on GitHub](https://github.com/UnitCircleInc/uclog/releases) as `uclog-element-N.iife.js`.

## What the page needs

- **A secure context**: `https://`, or `localhost` while you develop. Web Serial, WebHID and Web
  Bluetooth are only in Chrome and Edge.
- **In an iframe**, allow the device APIs: `<iframe src="…" allow="serial; hid; bluetooth">`.
- **A Content-Security-Policy**, if the page has one, must allow the script and the element's
  workers, which start from `blob:` URLs:

    ```
    script-src 'self' https://uclog.unitcircle.ca; worker-src blob:
    ```

- **Hubs**: a hub on another machine needs `wss://` (an `https://` page cannot open `ws://` to
  another host). A hub on the user's machine asks its operator before it lets in a page that is
  not on its `--allow-origin` list; start it with `--allow-origin https://your.site` to skip the
  question. Chrome also asks the user before a public site may reach this machine or the local
  network (*Local network access*).

## Logdata

The element decodes logs with the device's logdata, found by the image hash the device reports:

1. files the user loads (an ELF or a `.logdata` file, from the Device view);
2. the browser's cache of logdata it has used before;
3. the logdata service, when `logdata-domain` is set: `https://fw.unitcircle.ca/logdata/<domain>/<hash>`
   (another server with `logdata-url`).

The service only answers pages on the origins your account allows. If your account needs a
token to read logdata, the page sets it as a property (it is never an attribute, so it does not
appear in the HTML):

```js
document.querySelector('uc-log').logdataToken = tokenFromYourBackend;
```

A page can also hand over logdata itself, for example the build that matches the firmware it
ships:

```js
const r = await fetch('/firmware/app.logdata');
await el.loadLogdata({ name: 'app.logdata', data: new Uint8Array(await r.arrayBuffer()) });
```

## Scripting it

```js
const el = document.querySelector('uc-log');

// Connect: a hub, the simulated device, or a browser chooser (from a click).
await el.connect('ws://localhost:9000');
await el.connect('loopback:rate=20');
button.onclick = () => el.connect('serial');      // also 'hid' and 'ble'

el.send(61, 'help\r');                           // a line to the terminal port
await el.disconnect();

el.addEventListener('uc-connect', (e) => console.log('connected to', e.detail.uri));
el.addEventListener('uc-log', (e) => {            // with emit-logs
  for (const r of e.detail.records) if (r.level === 'ERROR') report(r.file, r.line, r.text);
});
```

Every attribute, property, method, event, part and colour is in the
[`<uc-log>` reference](reference/uc-log.md).

## Styling

Colours and shapes are CSS custom properties on the element (a value set by the page applies in
both light and dark), and its parts can be styled with `::part()`:

```css
uc-log {
  --uc-accent: #0b6e4f;
  --uc-radius: 4px;
  --uc-font: system-ui, sans-serif;
}
uc-log::part(header) { background: #f4f1ea; }
```

## Examples

Both are in the repository and tested on every change, so what you see here runs.

### A device portal

A stand-in for a product's support page ([`web/examples/embed`](https://github.com/UnitCircleInc/uclog/tree/main/web/examples/embed)):
its own buttons drive the element, its events are listed beside it, and a checkbox brands it. Its
server sends a strict Content-Security-Policy. To run it:

```sh
cd web
npx -y pnpm@12.10.1 install
npx -y pnpm@12.10.1 --filter @uclog/element build
node examples/embed/serve.mjs          # http://localhost:8080/
```

=== "index.html"

    ```html
    --8<-- "web/examples/embed/index.html"
    ```

=== "page.js"

    ```js
    --8<-- "web/examples/embed/page.js"
    ```

### The web app

The [web app](web-app.md) itself is `<uc-log>` and a page script
([`web/apps/demo`](https://github.com/UnitCircleInc/uclog/tree/main/web/apps/demo)), built with Vite from the package
(`import '@uclog/element'`). The page script adds what a standalone app needs: query parameters,
the sample logdata for the simulated device, and working offline.

=== "page.ts"

    ```ts
    --8<-- "web/apps/demo/src/page.ts"
    ```

=== "main.ts"

    ```ts
    --8<-- "web/apps/demo/src/main.ts"
    ```
