// The web app offline (plan 08 §3.1, task 9b):
// - the built app from a server: after one visit it loads with no network (sw.js); a new build is
//   announced, and "Reload" switches to it;
// - the one-file app from file://: a secure context with Web Serial, WebHID and Web Bluetooth, the
//   simulated device, and a hub, which refuses the "null" origin unless --allow-origin null.
// Builds web/apps/demo itself; the hub part needs native/target/{release,debug}/uclog-hub.

import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync } from 'node:fs';
import { createServer, type Server } from 'node:http';
import { tmpdir } from 'node:os';
import { extname, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { expect, test, type Page } from '@playwright/test';
import { decodeState } from './util.ts';

const DEMO = join(import.meta.dirname, '../apps/demo');
const HUB = ['release', 'debug'].map((p) => join(import.meta.dirname, `../../native/target/${p}/uclog-hub`)).find(existsSync);
const TYPES: Record<string, string> = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json' };

let app = '';
let single = '';
let server: Server;
let base = '';
let swBuild = 1; // bumped to stand for a new release: sw.js changes

function build(args: string[], out: string) {
  execFileSync('npx', ['vite', 'build', ...args, '--outDir', out, '--emptyOutDir'], { cwd: DEMO, stdio: 'ignore' });
}

test.beforeAll(async () => {
  app = mkdtempSync(join(tmpdir(), 'uclog-app-'));
  single = mkdtempSync(join(tmpdir(), 'uclog-single-'));
  build([], app);
  build(['--mode', 'single'], single);
  // The app under /app/, as on the site; sw.js gets a comment per "release".
  server = createServer((req, res) => {
    const path = new URL(req.url!, 'http://x').pathname;
    if (!path.startsWith('/app/')) return void res.writeHead(404).end();
    const file = join(app, path === '/app/' ? 'index.html' : path.slice(5));
    if (!existsSync(file)) return void res.writeHead(404).end();
    let body: Buffer | string = readFileSync(file);
    if (path === '/app/sw.js') body = `${body}\n// release ${swBuild}\n`;
    res.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream', 'cache-control': 'no-cache' }).end(body);
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${(server.address() as { port: number }).port}/app/`;
});

test.afterAll(() => server?.close());

async function simulated(page: Page) {
  await page.evaluate(() => (window as unknown as { ucLog: { connect(s: string): Promise<void> } }).ucLog.connect('loopback:rate=20'));
  await expect.poll(async () => (await decodeState(page))?.received ?? 0, { timeout: 15_000 }).toBeGreaterThan(5);
}

test('the app loads offline after one visit, and offers a new version', async ({ page, context }) => {
  await page.goto(base);
  await expect(page.locator('#app-bar')).toBeVisible();
  await page.evaluate(() => navigator.serviceWorker.ready);
  await context.setOffline(true);
  await page.reload();
  await expect(page.locator('uc-log')).toBeVisible();
  await simulated(page);
  await context.setOffline(false);

  // A new build on the server: announced while the page runs, used after "Reload".
  swBuild++;
  await page.reload(); // controlled by the first worker; it checks for an update
  await page.evaluate(() => navigator.serviceWorker.getRegistration().then((r) => r?.update()));
  await expect(page.locator('#app-update')).toBeVisible({ timeout: 15_000 });
  await page.locator('#app-update button').click();
  await page.waitForEvent('load');
  await expect(page.locator('#app-update')).toBeHidden();
  const after = await page.evaluate(() => caches.keys());
  expect(after).toHaveLength(1); // one build's cache (old ones are deleted on activate)
});

test('the one-file app from file://', async ({ page }) => {
  await page.goto(pathToFileURL(join(single, 'uclog-app.html')).href);
  const env = await page.evaluate(() => ({
    secure: isSecureContext,
    serial: 'serial' in navigator,
    hid: 'hid' in navigator,
    bluetooth: 'bluetooth' in navigator,
    origin: location.origin,
  }));
  console.log(`file:// page: ${JSON.stringify(env)}`);
  expect(env).toMatchObject({ secure: true, serial: true, hid: true, bluetooth: true });
  await expect(page.locator('#app-bar')).toHaveCount(0);
  await simulated(page);
});

for (const allowNull of [false, true]) {
  test(`the one-file app and a hub ${allowNull ? 'with' : 'without'} --allow-origin null`, async ({ page }) => {
    test.skip(!HUB, 'uclog-hub not built');
    const args = ['-d', 'loopback:rate=20', '--port', '0', '--bind', '127.0.0.1', '--no-mdns', '--no-registry', '--json-ready', '--exit-on-stdin-eof'];
    if (allowNull) args.push('--allow-origin', 'null');
    const hub: ChildProcess = spawn(HUB!, args, { stdio: ['pipe', 'pipe', 'inherit'] });
    try {
      const port = await new Promise<number>((resolve, reject) => {
        let out = '';
        hub.stdout!.on('data', (d) => {
          out += d;
          const line = out.split('\n').find((l) => l.startsWith('{'));
          if (line) resolve(JSON.parse(line).port);
        });
        hub.on('exit', (c) => reject(new Error(`hub exited ${c}`)));
      });
      await page.goto(pathToFileURL(join(single, 'uclog-app.html')).href);
      void page.evaluate((u) => (window as unknown as { ucLog: { connect(s: string): Promise<void> } }).ucLog.connect(u).catch(() => {}), `ws://127.0.0.1:${port}`);
      if (allowNull) {
        await expect.poll(async () => (await decodeState(page))?.received ?? 0, { timeout: 15_000 }).toBeGreaterThan(5);
      } else {
        await expect(page.locator('footer')).toContainText('--allow-origin null', { timeout: 15_000 });
      }
    } finally {
      hub.stdin?.end();
      hub.kill();
    }
  });
}
