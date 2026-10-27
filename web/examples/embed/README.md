# Embedding example

A stand-in for a third-party site that embeds `<uc-log>` from the built bundles, served with a
strict Content-Security-Policy.

```sh
cd web
npx -y pnpm@12.10.1 install
npx -y pnpm@12.10.1 --filter @uclog/element build    # packages/element/dist
node examples/embed/serve.mjs                         # http://localhost:8080/
```

| URL | Loads |
|---|---|
| `http://localhost:8080/` | the ES module (`uclog-element.js`; uPlot loads with the first plot) |
| `/?bundle=iife` | the single classic script (`uclog-element.iife.js`) |
| `/?worker-base=1` | the ES module with the workers as files (`worker-base="/uclog/workers"`), for a CSP without `worker-src blob:` |

Files:

- `index.html`: the page, with `<uc-log>` and its attributes.
- `page.js`: loads the bundle, calls `connect()`, `disconnect()`, `send()` and `loadLogdata()`,
  and lists the element's events.
- `serve.mjs`: the server. It serves `packages/element/dist` at `/uclog/` and
  `spec/vectors/sample.logdata` at `/sample.logdata`, and sets the CSP.

To try it:

1. **Load sample.logdata**, then **Simulated device**.
2. Plots: type `plot` in the Terminal view.
3. **Serial…** opens Chrome's port chooser, for example for the NUCLEO. Close any hub or TUI using
   the port first, and load its logdata from the Device view (*Load logdata…*, the `app.elf`).
4. **Hub** connects to `uclog hub -d … --port 9000`. `localhost` origins are allowed by default.
5. **branding** sets `--uc-accent` and `--uc-radius` on the element.

On your own site, copy `uclog-element.js` and its chunks (or just `uclog-element.iife.js`) next to
your page. Then add:

```html
<script type="module" src="uclog-element.js"></script>
<uc-log logdata-domain="acme.example" hub-url="ws://localhost:9000"></uc-log>
```

The page must be a secure context (https, or localhost). Its CSP needs `worker-src blob:`, or
`worker-base` pointing at the copied `workers/` folder. To embed it in an iframe, use
`allow="serial; hid; bluetooth"`. The attributes, methods and events are on the docs site
(*Embedding*, and the generated *Element reference*), which also shows this example's source.
`e2e/embed.spec.ts` runs it, with each way of loading the bundle.
