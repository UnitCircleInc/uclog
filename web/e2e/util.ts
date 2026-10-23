import { expect, type Page } from '@playwright/test';

/** Errors the page logs or throws (each test asserts there are none). */
export function watchErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(`console: ${m.text()}`);
  });
  return errors;
}

export const ucLog = (page: Page) => page.locator('uc-log');

export async function connectSimulated(page: Page, rate = 20) {
  await page.goto('/');
  await page.getByRole('button', { name: 'Connect…' }).click();
  // Use a specific rate: the picker's demo button is rate=5.
  await page.evaluate((r) => (window as unknown as { ucLog: { connect(s: string): Promise<void> } }).ucLog.connect(`loopback:rate=${r}`), rate);
  await page.getByRole('button', { name: 'Close' }).click();
  await expect(page.locator('footer')).toContainText('loopback (loopback)');
}

/** The decode worker's state as the client last saw it. */
export async function decodeState(page: Page) {
  return page.evaluate(() => (window as unknown as { ucLog: { client: { decodeState: unknown } } }).ucLog.client.decodeState as {
    received: number;
    visible: number;
    errorCounts: Record<string, number>;
    plots: { name: string }[];
  } | null);
}
