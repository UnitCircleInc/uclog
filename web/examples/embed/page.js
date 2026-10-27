// The host page's own code: load the bundle, drive the element, listen to its events.
// ?bundle=iife loads the single-file classic script instead of the ES module. ?worker-base=1
// uses the worker files (no blob: workers, for sites whose CSP forbids them).

const q = new URLSearchParams(location.search);
const el = document.querySelector('uc-log');
if (q.has('worker-base')) el.setAttribute('worker-base', '/uclog/workers');

const script = document.createElement('script');
if (q.get('bundle') === 'iife') script.src = '/uclog/uclog-element.iife.js';
else {
  script.type = 'module';
  script.src = '/uclog/uclog-element.js';
}
document.head.append(script);

const events = document.getElementById('events');
function log(type, detail) {
  const d = document.createElement('div');
  d.textContent = `${new Date().toLocaleTimeString()} ${type} ${detail === undefined ? '' : JSON.stringify(detail)}`;
  events.insertBefore(d, events.children[1] ?? null);
  while (events.children.length > 200) events.lastChild.remove();
}

el.addEventListener('uc-ready', () => log('uc-ready'));
el.addEventListener('uc-connect', (e) => log('uc-connect', e.detail));
el.addEventListener('uc-disconnect', (e) => log('uc-disconnect', e.detail));
el.addEventListener('uc-error', (e) => log('uc-error', e.detail));
el.addEventListener('uc-device-info', (e) => log('uc-device-info', e.detail.fields));
// With emit-logs: decoded records, in batches. Here, warnings and worse.
const LOUD = new Set(['WARN', 'ERROR', 'FATAL', 'PANIC']);
el.addEventListener('uc-log', (e) => {
  for (const r of e.detail.records) if (LOUD.has(r.level)) log('uc-log', `${r.level} ${r.file}:${r.line} ${r.text}`);
});

const on = (id, f) => document.getElementById(id).addEventListener('click', () => f().catch?.((e) => log('error', e.message)));
on('sim', () => el.connect('loopback:rate=5'));
on('serial', () => el.connect('serial')); // opens the browser's chooser (needs a click)
on('hubgo', () => el.connect(document.getElementById('hub').value));
on('disc', () => el.disconnect());
on('hello', async () => el.send(61, 'echo hello\r'));
on('logdata', async () => {
  // The simulated device's logdata. For a real device, load its .logdata or ELF the same way, or
  // set logdata-domain to look it up by app_hash.
  const data = new Uint8Array(await (await fetch('/sample.logdata')).arrayBuffer());
  const r = await el.loadLogdata({ name: 'sample.logdata', data });
  log('loadLogdata', r);
});
document.getElementById('brand').addEventListener('change', (e) => el.classList.toggle('branded', e.target.checked));
