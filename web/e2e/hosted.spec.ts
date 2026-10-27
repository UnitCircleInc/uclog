// The web app as hosted at https://uclog.unitcircle.ca (plan 08 §3, task 10), against a hub on this
// machine: the production build is served under that origin (requests are answered from the
// build), so the page's Origin is the real one.
// - Chrome's Local Network Access blocks a public page's ws://127.0.0.1 until the site has the
//   "local-network-access" permission (Chrome asks the user; Playwright would deny): without it the
//   connection never opens, and the app says why.
// - With it, the hub allows the hosted origin by default (--allow-origin), and not another public
//   origin.
// Needs native/target/{release,debug}/uclog-hub; skipped without it.

import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { decodeState } from './util.ts';

const HUB = ['release', 'debug'].map((p) => join(import.meta.dirname, `../../native/target/${p}/uclog-hub`)).find(existsSync);
const TYPES: Record<string, string> = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.png': 'image/png', '.json': 'application/json' };

let hub: ChildProcess | undefined;
let url = '';
let dist = '';

test.beforeAll(async () => {
  test.skip(!HUB, 'uclog-hub not built');
  dist = mkdtempSync(join(tmpdir(), 'uclog-hosted-'));
  execFileSync('npx', ['vite', 'build', '--outDir', dist, '--emptyOutDir'], { cwd: join(import.meta.dirname, '../apps/demo'), stdio: 'ignore' });
  hub = spawn(HUB!, ['-d', 'loopback:rate=20', '--port', '0', '--bind', '127.0.0.1', '--no-mdns', '--no-registry', '--json-ready', '--exit-on-stdin-eof'], {
    stdio: ['pipe', 'pipe', 'inherit'],
  });
  const port = await new Promise<number>((resolve, reject) => {
    let out = '';
    hub!.stdout!.on('data', (d) => {
      out += d;
      const line = out.split('\n').find((l) => l.startsWith('{'));
      if (line) resolve(JSON.parse(line).port);
    });
    hub!.on('exit', (c) => reject(new Error(`hub exited ${c}`)));
  });
  url = `ws://127.0.0.1:${port}`;
});

test.afterAll(() => {
  hub?.stdin?.end();
  hub?.kill();
});

// Serve the build as `origin`, with Chrome's local network permission or without.
async function open(page: Page, origin: string, localNetwork = true) {
  if (localNetwork) await page.context().grantPermissions(['local-network-access'], { origin });
  await page.route(`${origin}/**`, (route) => {
    const path = new URL(route.request().url()).pathname;
    const file = join(dist, path === '/' ? 'index.html' : path);
    if (!existsSync(file)) return route.fulfill({ status: 404 });
    return route.fulfill({ body: readFileSync(file), contentType: TYPES[extname(file)] ?? 'application/octet-stream' });
  });
  await page.goto(`${origin}/`);
}

test('the hosted origin is allowed by a local hub', async ({ page }) => {
  await open(page, 'https://uclog.unitcircle.ca');
  await page.evaluate((u) => (window as unknown as { ucLog: { connect(s: string): Promise<void> } }).ucLog.connect(u), url);
  await expect(page.locator('footer')).toContainText(url, { timeout: 15_000 });
  await expect.poll(async () => (await decodeState(page))?.received ?? 0, { timeout: 15_000 }).toBeGreaterThan(5);
  await expect(page.locator('footer')).not.toContainText('approval');
  await page.evaluate(() => (window as unknown as { ucLog: { disconnect(): Promise<void> } }).ucLog.disconnect());
});

// This hub has no console to ask its operator on (stdin is the test's pipe), so it turns the page
// away (1008); with a console it would ask, and the page would wait for the approval.
test('another public origin is not let in', async ({ page }) => {
  await open(page, 'https://other.example');
  void page.evaluate((u) => (window as unknown as { ucLog: { connect(s: string): Promise<void> } }).ucLog.connect(u).catch(() => {}), url);
  await expect(page.locator('footer')).toContainText('1008 rejected', { timeout: 15_000 });
});

test('without the local network permission the app explains the failure', async ({ page }) => {
  await open(page, 'https://uclog.unitcircle.ca', false);
  void page.evaluate((u) => (window as unknown as { ucLog: { connect(s: string): Promise<void> } }).ucLog.connect(u).catch(() => {}), url);
  await expect(page.locator('footer')).toContainText('Local network access', { timeout: 15_000 });
});
