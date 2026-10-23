// Throughput (plan 02 §8 task 11): at least 10 000 logs/s reach the log store, and the main thread
// stays responsive (no long tasks) while the table follows.

import { expect, test } from '@playwright/test';
import { connectSimulated, decodeState } from './util.ts';

test('20 000-log burst: rate and main-thread long tasks', async ({ page }) => {
  await connectSimulated(page, 0);
  await page.evaluate(() => {
    const w = window as unknown as { longTasks: number[] };
    w.longTasks = [];
    new PerformanceObserver((l) => l.getEntries().forEach((e) => w.longTasks.push(e.duration))).observe({ type: 'longtask', buffered: false });
  });
  await expect.poll(async () => (await decodeState(page))?.received ?? 0).toBeGreaterThan(0);
  const before = (await decodeState(page))!.received;
  const N = 20_000;
  const t0 = Date.now();
  await page.evaluate(() => (window as unknown as { ucLog: { send(p: number, d: string): void } }).ucLog.send(61, 'burst 20000\r'));
  await expect.poll(async () => (await decodeState(page))!.received - before, { timeout: 20_000, intervals: [50] }).toBeGreaterThanOrEqual(N);
  const secs = (Date.now() - t0) / 1000;
  const longTasks = await page.evaluate(() => (window as unknown as { longTasks: number[] }).longTasks);
  const st = (await decodeState(page))!;
  console.log(`${N} logs in ${secs.toFixed(2)} s = ${Math.round(N / secs)} logs/s; main-thread long tasks: ${longTasks.map((d) => Math.round(d)).join(', ') || 'none'}`);
  expect(st.errorCounts).toEqual({});
  expect(N / secs).toBeGreaterThan(10_000);
  expect(Math.max(0, ...longTasks)).toBeLessThan(100);
  // The table followed: scrolled to the bottom, with the last row rendered.
  await expect
    .poll(() =>
      page.evaluate(() => {
        const v = document.querySelector('uc-log')!.shadowRoot!.querySelector('uc-log-view')!.shadowRoot!;
        const el = v.querySelector('.scroll') as HTMLElement;
        const rows = [...v.querySelectorAll('.r')] as HTMLElement[];
        const last = Math.max(...rows.map((r) => parseFloat(r.style.top)));
        return el.scrollTop + el.clientHeight >= el.scrollHeight - 2 && last + 20 >= el.scrollHeight - 2;
      }),
    )
    .toBe(true);
});

test('scrolling up stops following; back at the bottom follows again', async ({ page }) => {
  await connectSimulated(page, 200);
  await expect.poll(async () => (await decodeState(page))?.visible ?? 0).toBeGreaterThan(200);
  const table = page.locator('uc-log-view .scroll');
  await table.hover();
  await page.mouse.wheel(0, -2000);
  await expect(page.getByRole('button', { name: '↓ Follow' })).toBeVisible();
  const top = await table.evaluate((el) => el.scrollTop);
  await page.waitForTimeout(500);
  expect(await table.evaluate((el) => el.scrollTop)).toBe(top);
  await page.getByRole('button', { name: '↓ Follow' }).click();
  await expect(page.getByRole('button', { name: '↓ Follow' })).toBeHidden();
});
