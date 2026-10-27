# @uclog/element

`<uc-log>`, the [ucLog](https://uclog.unitcircle.ca/) web client as one custom element: connect
to a device over Web Serial, WebHID, Web Bluetooth or a ucLog hub, and show its decoded logs,
terminal, commands, plots and device info in your own page.

```sh
npm install @uclog/element
```

```html
<script type="module">import '@uclog/element';</script>
<uc-log></uc-log>
```

Or with no build step, one classic script:

```html
<script src="https://uclog.unitcircle.ca/element/latest/uclog-element.iife.js"></script>
```

The package has the ES module (`@uclog/element`), the one-script build (`@uclog/element/iife`),
the two workers as files (`@uclog/element/workers/*`, for `worker-base`) and the Custom Elements
Manifest (`dist/custom-elements.json`).

Docs: [embedding `<uc-log>`](https://uclog.unitcircle.ca/latest/embedding/) and the
[element reference](https://uclog.unitcircle.ca/latest/reference/uc-log/). Apache-2.0.
