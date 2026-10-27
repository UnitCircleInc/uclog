// Hardware, through the native hub over WebSocket (Web Serial's chooser cannot be automated):
//   UCLOG_TEST_DEVICE=serial:/dev/cu.usbmodem… UCLOG_TEST_ELF=../firmware/nucleo-l452re-p/build/app.elf \
//   npx playwright test hw
// The ELF's .logdata_cbor is loaded in the page, then `test` and `burst` must decode cleanly, and
// `plot` must draw.

import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';
import { decodeState, watchErrors } from './util.ts';

const DEV = process.env.UCLOG_TEST_DEVICE;
const ELF = process.env.UCLOG_TEST_ELF;
const HUB = ['release', 'debug'].map((p) => join(import.meta.dirname, `../../native/target/${p}/uclog-hub`)).find(existsSync);

let hub: ChildProcess | undefined;
let url = '';

test.beforeAll(async () => {
  test.skip(!DEV || !ELF || !HUB, 'set UCLOG_TEST_DEVICE and UCLOG_TEST_ELF, and build uclog-hub');
  hub = spawn(HUB!, ['-d', DEV!, '--port', '0', '--bind', '127.0.0.1', '--no-mdns', '--no-registry', '--json-ready', '--exit-on-stdin-eof'], {
    stdio: ['pipe', 'pipe', 'inherit'],
  });
  url = await new Promise<string>((resolve, reject) => {
    let out = '';
    hub!.stdout!.on('data', (d) => {
      out += d;
      const line = out.split('\n').find((l) => l.startsWith('{'));
      if (line) resolve(`ws://127.0.0.1:${JSON.parse(line).port}`);
    });
    hub!.on('exit', (c) => reject(new Error(`hub exited ${c}`)));
  });
});

test.afterAll(() => {
  hub?.stdin?.end();
  hub?.kill();
});

test('real device through the hub: device info, ELF logdata, test, burst, plot', async ({ page }) => {
  test.setTimeout(120_000);
  const errors = watchErrors(page);
  await page.goto('/');
  const elf = [...readFileSync(ELF!)];
  await page.evaluate((b) => (window as unknown as { ucLog: { loadLogdata(f: object): Promise<unknown> } }).ucLog.loadLogdata({ name: 'app.elf', data: new Uint8Array(b) }), elf);
  await page.evaluate((u) => (window as unknown as { ucLog: { connect(s: string): Promise<void> } }).ucLog.connect(u), url);
  await expect(page.locator('footer')).toContainText('logdata: manual app.elf', { timeout: 15_000 });
  await page.getByRole('tab', { name: 'Terminal' }).click();
  const input = page.getByLabel('Terminal command');
  await input.fill('test');
  await input.press('Enter');
  await expect.poll(async () => (await decodeState(page))?.received ?? 0, { timeout: 15_000 }).toBeGreaterThan(20);
  const before = (await decodeState(page))!.received;
  await input.fill('burst 5000');
  await input.press('Enter');
  await expect(page.locator('.xterm-rows')).toContainText('burst of 5000 logs', { timeout: 30_000 });
  await expect.poll(async () => (await decodeState(page))!.received - before, { timeout: 30_000 }).toBeGreaterThanOrEqual(5000);
  const st = (await decodeState(page))!;
  console.log(`received ${st.received}, error records ${JSON.stringify(st.errorCounts)}`);
  expect(st.errorCounts).toEqual({});
  // Plots through the device library's plot API (log_plot.h).
  await input.fill('plot 100');
  await input.press('Enter');
  await expect(page.locator('.xterm-rows')).toContainText('plot "waves": 100 of 100 points', { timeout: 15_000 });
  await expect.poll(async () => (await decodeState(page))!.plots.map((p) => p.name)).toContain('waves');
  await page.getByRole('tab', { name: 'Plots' }).click();
  await expect(page.locator('uc-line-plot .u-legend').first()).toContainText('sin*cos');
  await page.getByRole('tab', { name: 'Device' }).click();
  await expect(page.locator('uc-device-info')).toContainText('app_hash');
  expect(errors).toEqual([]);
});
