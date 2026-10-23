// A static server for the embedding example (a stand-in for "your web site").
//
//   node examples/embed/serve.mjs [--port 8080]
//
// Serves this folder at /, the built <uc-log> bundles (packages/element/dist) at /uclog/, and
// spec/vectors/sample.logdata at /sample.logdata, with a strict Content-Security-Policy like a
// production site would send. http://localhost counts as a secure context, so Web Serial, WebHID
// and Web Bluetooth work there.

import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const web = resolve(here, '../..');
const roots = { '/uclog/': join(web, 'packages/element/dist'), '/': here };
const port = Number(process.argv[process.argv.indexOf('--port') + 1]) || 8080;
const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.map': 'application/json', '.css': 'text/css' };

// What the page may do: its own scripts, the inlined workers (blob:), and WebSocket to hubs.
const CSP = [
  "default-src 'self'",
  "script-src 'self'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data:",
  'worker-src blob: \'self\'',
  "connect-src 'self' ws: wss: https://fw.unitcircle.ca",
].join('; ');

const SAMPLE = join(web, '../spec/vectors/sample.logdata');

/** The file for a URL path, never outside the roots. */
function fileFor(path) {
  if (path === '/sample.logdata') return SAMPLE;
  const [prefix, root] = Object.entries(roots).find(([p]) => path.startsWith(p));
  const f = resolve(root, `.${normalize(`/${path.slice(prefix.length)}`)}`);
  if (!f.startsWith(root)) return null;
  return path.endsWith('/') ? join(f, 'index.html') : f;
}

createServer(async (req, res) => {
  const file = fileFor(decodeURIComponent(new URL(req.url, 'http://x').pathname));
  try {
    if (!file) throw new Error('outside');
    const body = await readFile(file);
    res.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream', 'content-security-policy': CSP });
    res.end(body);
  } catch {
    res.writeHead(404).end('not found');
  }
}).listen(port, () => {
  console.log(`embedding example on http://localhost:${port}/  (IIFE bundle: http://localhost:${port}/?bundle=iife)`);
});
