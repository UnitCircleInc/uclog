// Screenshots of every view for review and docs: UCLOG_SHOTS=dir npx playwright test screens.
import { test } from '@playwright/test';
import { connectSimulated } from './util.ts';

const DIR = process.env.UCLOG_SHOTS;

for (const scheme of ['light', 'dark'] as const) {
  test(`screens ${scheme}`, async ({ page }) => {
    test.skip(!DIR, 'set UCLOG_SHOTS');
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto('/');
    await page.getByRole('button', { name: 'Connect…' }).click();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${DIR}/${scheme}-picker.png` });
    await page.getByRole('button', { name: 'Close' }).click();
    await connectSimulated(page, 20);
    await page.evaluate(() => (window as unknown as { ucLog: { send(p: number, d: string): void } }).ucLog.send(61, 'help\r'));
    await page.evaluate(() => (window as unknown as { ucLog: { send(p: number, d: string): void } }).ucLog.send(61, 'sh\r'));
    await page.evaluate(() => (window as unknown as { ucLog: { send(p: number, d: string): void } }).ucLog.send(61, 'plot\r'));
    await page.waitForTimeout(2500);
    await page.screenshot({ path: `${DIR}/${scheme}-logs.png` });
    for (const tab of ['Terminal', 'Plots', 'Device', 'FW update', 'Commands']) {
      await page.getByRole('tab', { name: tab }).click();
      await page.waitForTimeout(600);
      await page.screenshot({ path: `${DIR}/${scheme}-${tab.toLowerCase().replace(' ', '-')}.png` });
    }
    await page.getByRole('tab', { name: 'Plots' }).click();
    await page.getByRole('tab', { name: 'spectrum' }).click();
    await page.waitForTimeout(600);
    await page.screenshot({ path: `${DIR}/${scheme}-spectrum.png` });
    await page.getByRole('tab', { name: 'grid' }).click();
    await page.waitForTimeout(600);
    await page.screenshot({ path: `${DIR}/${scheme}-grid.png` });
  });
}
