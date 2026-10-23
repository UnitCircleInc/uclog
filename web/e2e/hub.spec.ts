// Through the native hub over WebSocket (protocol.md §8): the hub's loopback device, the device
// info replay, a second client turned away with close code 4001 ("hub busy").
// Needs native/target/{release,debug}/uclog-hub; skipped without it.

import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';
import { decodeState, watchErrors } from './util.ts';

const HUB = ['release', 'debug'].map((p) => join(import.meta.dirname, `../../native/target/${p}/uclog-hub`)).find(existsSync);

let hub: ChildProcess | undefined;
let url = '';

test.beforeAll(async () => {
  test.skip(!HUB, 'uclog-hub not built');
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

test('connect through the hub; a second client is busy', async ({ page, browser }) => {
  const errors = watchErrors(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Connect…' }).click();
  await page.getByLabel('Hub URL').fill(url);
  await page.getByRole('button', { name: 'Connect to hub' }).click();
  await expect(page.locator('footer')).toContainText(url);
  // The hub replays the device info, so the logdata is found and logs decode.
  await expect(page.locator('footer')).toContainText('logdata: manual');
  await expect.poll(async () => (await decodeState(page))?.received ?? 0).toBeGreaterThan(5);
  expect((await decodeState(page))!.errorCounts).toEqual({});

  // Terminal through the hub.
  await page.getByRole('tab', { name: 'Terminal' }).click();
  await page.getByLabel('Terminal command').fill('echo via the hub');
  await page.getByLabel('Terminal command').press('Enter');
  await expect(page.locator('.xterm-rows')).toContainText('via the hub');

  // A second page: busy.
  const other = await (await browser.newContext()).newPage();
  await other.goto('/');
  await other.evaluate((u) => (window as unknown as { ucLog: { connect(s: string): Promise<void> } }).ucLog.connect(u), url);
  await expect(other.locator('footer')).toContainText('hub busy');

  // The first disconnects; the second can connect, and the hub remembered the URL for it.
  await page.getByRole('button', { name: 'Disconnect' }).click();
  await other.evaluate((u) => (window as unknown as { ucLog: { connect(s: string): Promise<void> } }).ucLog.connect(u), url);
  await expect.poll(async () => (await decodeState(other))?.received ?? 0).toBeGreaterThan(5);
  await page.reload();
  await page.getByRole('button', { name: 'Connect…' }).click();
  await expect(page.locator('uc-device-picker li')).toContainText(url);
  expect(errors).toEqual([]);
});
