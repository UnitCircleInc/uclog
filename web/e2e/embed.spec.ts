// The built <uc-log> bundles on a third-party page (plan 02 §7): attributes, events, methods,
// the IIFE with inlined workers under a CSP that allows `worker-src blob:`, and the ESM bundle
// with `worker-base` (no blob: needed). Then examples/embed itself, as the docs show it, from its
// own server (serve.mjs, with its CSP), with each way of loading the bundle.

import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { watchErrors } from './util.ts';

const DIST = join(import.meta.dirname, '../packages/element/dist'); // built by global-setup.ts
const SAMPLE = readFileSync(join(import.meta.dirname, '../../spec/vectors/sample.logdata'));

/** Serve a page and the bundle files under http://localhost:5199/embed/ (a secure context). */
async function serve(page: Page, html: string, csp: string) {
  await page.route('**/embed/**', async (route) => {
    const path = new URL(route.request().url()).pathname.replace(/^\/embed\//, '');
    if (path === '' || path === 'index.html') {
      return route.fulfill({ contentType: 'text/html', body: html, headers: { 'content-security-policy': csp } });
    }
    if (path === 'sample.logdata') return route.fulfill({ body: SAMPLE, contentType: 'application/octet-stream' });
    try {
      return route.fulfill({ body: readFileSync(join(DIST, path)), contentType: 'text/javascript' });
    } catch {
      return route.fulfill({ status: 404 });
    }
  });
}

const PAGE_SCRIPT = `
  const el = document.querySelector('uc-log');
  window.events = [];
  for (const t of ['uc-connect', 'uc-disconnect', 'uc-device-info', 'uc-log', 'uc-error'])
    el.addEventListener(t, (e) => window.events.push([t, t === 'uc-log' ? e.detail.records.length : e.detail]));
  fetch('sample.logdata').then((r) => r.arrayBuffer()).then((b) => el.loadLogdata({ name: 'sample.logdata', data: new Uint8Array(b) }));
`;

async function exercise(page: Page) {
  await expect(page.locator('uc-log')).toBeVisible();
  // panels="log,terminal": only those two; layout=grid shows both at once.
  await expect(page.locator('uc-log section')).toHaveCount(2);
  await expect(page.locator('uc-log')).toHaveAttribute('data-dark', '');
  await page.evaluate(() => (document.querySelector('uc-log') as unknown as { connect(s: string): Promise<void> }).connect('loopback:rate=50'));
  await expect(page.getByText('Hello World! loopback start')).toBeVisible();
  await page.evaluate(() => (document.querySelector('uc-log') as unknown as { send(p: number, d: string): void }).send(61, 'echo embedded\r'));
  await expect(page.locator('.xterm-rows')).toContainText('embedded');
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { events: [string, unknown][] }).events.map((e) => e[0])))
    .toEqual(expect.arrayContaining(['uc-connect', 'uc-device-info', 'uc-log']));
  await page.evaluate(() => (document.querySelector('uc-log') as unknown as { disconnect(): Promise<void> }).disconnect());
  await expect.poll(() => page.evaluate(() => (window as unknown as { events: [string, unknown][] }).events.map((e) => e[0]))).toContain('uc-disconnect');
}

test('IIFE bundle, inlined workers, CSP worker-src blob:', async ({ page }) => {
  const errors = watchErrors(page);
  await serve(
    page,
    `<!doctype html><meta charset="utf-8"><link rel="icon" href="data:,">
     <body style="height:100vh;margin:0">
     <uc-log panels="log,terminal" layout="grid" theme="dark" emit-logs storage-key="embed-iife" style="height:600px"></uc-log>
     <script src="uclog-element.iife.js"></script>
     <script>${PAGE_SCRIPT}</script></body>`,
    "default-src 'self'; img-src data:; script-src 'self' 'unsafe-inline'; worker-src blob:; style-src 'self' 'unsafe-inline'; connect-src 'self' ws: wss:",
  );
  await page.goto('/embed/');
  await exercise(page);
  // The documented parts reach into the views (exportparts): the page can style them.
  await page.addStyleTag({ content: 'uc-log::part(table), uc-log::part(terminal), uc-log::part(toolbar) { outline: 3px solid rgb(1, 2, 3); }' });
  const outlined = await page.evaluate(() => {
    const deep = (root: Document | ShadowRoot, sel: string): Element[] =>
      [...root.querySelectorAll(sel), ...[...root.querySelectorAll('*')].flatMap((e) => (e.shadowRoot ? deep(e.shadowRoot, sel) : []))];
    return ['table', 'terminal', 'toolbar'].map((p) => deep(document, `[part~="${p}"]`).map((e) => getComputedStyle(e).outlineColor));
  });
  expect(outlined).toEqual([['rgb(1, 2, 3)'], ['rgb(1, 2, 3)'], ['rgb(1, 2, 3)', 'rgb(1, 2, 3)']]);
  expect(errors).toEqual([]);
});

test('ESM bundle with worker-base (no blob: workers)', async ({ page }) => {
  const errors = watchErrors(page);
  await serve(
    page,
    `<!doctype html><meta charset="utf-8"><link rel="icon" href="data:,">
     <body style="height:100vh;margin:0">
     <uc-log panels="log,terminal" layout="grid" theme="dark" emit-logs storage-key="embed-esm" worker-base="workers" style="height:600px"></uc-log>
     <script type="module" src="uclog-element.js"></script>
     <script type="module">${PAGE_SCRIPT}</script></body>`,
    "default-src 'self'; img-src data:; script-src 'self' 'unsafe-inline'; worker-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' ws: wss:",
  );
  await page.goto('/embed/');
  await exercise(page);
  expect(errors).toEqual([]);
});

for (const variant of ['', '?bundle=iife', '?worker-base=1']) {
  test(`examples/embed${variant}`, async ({ page }) => {
    const errors = watchErrors(page);
    const server = spawn('node', [join(import.meta.dirname, '../examples/embed/serve.mjs'), '--port', '0'], { stdio: ['ignore', 'pipe', 'inherit'] });
    try {
      const url = await new Promise<string>((resolve, reject) => {
        server.stdout.on('data', (d) => {
          const m = /http:\/\/localhost:\d+\//.exec(String(d));
          if (m) resolve(m[0]);
        });
        server.on('exit', (c) => reject(new Error(`serve.mjs exited ${c}`)));
      });
      await page.goto(url + variant);
      const events = page.locator('#events');
      await expect(events).toContainText('uc-ready');
      await page.getByRole('button', { name: 'Load sample.logdata' }).click();
      await expect(events).toContainText('loadLogdata');
      await page.getByRole('button', { name: 'Simulated device' }).click();
      await expect(events).toContainText('uc-connect');
      await expect(events).toContainText('uc-device-info');
      await expect(events).toContainText(/uc-log "WARN src\/main\.c:\d+ temp=/, { timeout: 15_000 }); // warnings only
      await page.getByRole('button', { name: 'send "echo hello"' }).click();
      await page.locator('uc-log').getByRole('tab', { name: 'Terminal' }).click();
      await expect(page.locator('.xterm-rows')).toContainText('hello');
      await page.getByLabel('branding').check();
      await expect.poll(() => page.locator('uc-log').evaluate((e) => getComputedStyle(e).getPropertyValue('--uc-accent').trim())).toBe('#0f766e');
      await page.locator('#disc').click(); // the page's button (the element has one too)
      await expect(events).toContainText('uc-disconnect');
      expect(errors).toEqual([]);
    } finally {
      server.kill();
    }
  });
}
