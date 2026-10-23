// The demo page with the simulated device: every view, in Chrome.

import { expect, test } from '@playwright/test';
import { connectSimulated, decodeState, watchErrors } from './util.ts';

test('logs, terminal, commands, plots, device info, export', async ({ page }) => {
  const errors = watchErrors(page);
  await connectSimulated(page);

  // Logs decode with the sample logdata (loaded under the simulated app_hash).
  await expect(page.getByText('Hello World! loopback start')).toBeVisible();
  await expect(page.locator('footer')).toContainText('logdata: manual');
  await expect.poll(async () => (await decodeState(page))?.received ?? 0).toBeGreaterThan(10);
  expect((await decodeState(page))!.errorCounts).toEqual({});

  // Level filter: ≥ WARN hides the INFO lines.
  await page.getByLabel('Minimum level').selectOption({ label: '≥ WARN' });
  await expect(page.getByText('Hello World! loopback start')).toBeHidden();
  await page.getByRole('button', { name: 'Clear filter' }).click();
  await expect(page.getByText('Hello World! loopback start')).toBeVisible();

  // Terminal: typed line and history.
  await page.getByRole('tab', { name: 'Terminal' }).click();
  const input = page.getByLabel('Terminal command');
  await input.fill('echo hi from the web');
  await input.press('Enter');
  await expect(page.locator('.xterm-rows')).toContainText('hi from the web');
  await input.press('ArrowUp');
  await expect(input).toHaveValue('echo hi from the web');

  // Semihost stdin: `read` asks for a line.
  await input.fill('read');
  await input.press('Enter');
  const stdin = page.getByLabel('Line for the device stdin');
  await expect(stdin).toBeVisible();
  await stdin.fill('typed');
  await stdin.press('Enter');
  await expect(page.locator('.xterm-rows')).toContainText('read: n=6');

  // Commands view: free line on port 0.
  await page.getByRole('tab', { name: 'Commands' }).click();
  await page.getByLabel('Command', { exact: true }).fill('echo on port zero');
  await page.getByLabel('Command', { exact: true }).press('Enter');
  await expect(page.locator('uc-commands')).toContainText('on port zero');

  // Plots: start them from the terminal, check each kind draws.
  await page.getByRole('tab', { name: 'Terminal' }).click();
  await input.fill('plot');
  await input.press('Enter');
  await page.getByRole('tab', { name: 'Plots' }).click();
  await expect(page.getByRole('tab', { name: 'loopback' })).toBeVisible();
  await expect(page.locator('uc-line-plot .u-over')).toBeVisible();
  await expect(page.locator('uc-line-plot .u-legend')).toContainText('sin');
  // Follow stays on and the x axis keeps up with the data; a drag zooms and stops following.
  const follow = page.locator('uc-line-plot').getByLabel('Follow');
  await expect(follow).toBeChecked();
  const xmax = () => page.locator('uc-line-plot').evaluate((el) => (el as unknown as { u: { scales: { x: { max: number } } } }).u.scales.x.max);
  const x1 = await xmax();
  await expect.poll(xmax).toBeGreaterThan(x1 + 0.2);
  const box = (await page.locator('uc-line-plot .u-over').boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height / 2, { steps: 5 });
  await page.mouse.up();
  await expect(follow).not.toBeChecked();
  await page.locator('uc-line-plot .u-over').dblclick();
  await expect(follow).toBeChecked();
  await page.getByRole('tab', { name: 'spectrum' }).click();
  await expect(page.locator('uc-grid-plot canvas.map')).toBeVisible();
  await page.getByRole('tab', { name: 'grid' }).click();
  await expect(page.locator('uc-grid-plot canvas.map')).toHaveJSProperty('width', 8);

  // Device info.
  await page.getByRole('tab', { name: 'Device' }).click();
  await expect(page.locator('uc-device-info')).toContainText('board');
  await expect(page.locator('uc-device-info')).toContainText('5a5a5a5a');

  // Export all → a zip with the logs, terminal and three plots.
  await page.getByRole('button', { name: '⋯' }).click();
  const dl = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Export all (zip)' }).click();
  const file = await dl;
  expect(file.suggestedFilename()).toMatch(/^uclog-\d{8}-\d{6}\.zip$/);
  const buf = await (await file.createReadStream()).toArray();
  const zipText = Buffer.concat(buf).toString('latin1');
  for (const n of ['-logs.csv', '-terminal.txt', '-plot-loopback.csv', '-plot-spectrum.csv', '-plot-grid.csv']) expect(zipText).toContain(n);

  await page.getByRole('button', { name: 'Disconnect' }).click();
  await expect(page.locator('footer')).toContainText('disconnected');
  expect(errors).toEqual([]);
});

test('settings: command buttons with $args', async ({ page }) => {
  const errors = watchErrors(page);
  await connectSimulated(page, 0);
  await page.getByRole('button', { name: '⋯' }).click();
  await page.getByRole('button', { name: 'Settings…' }).click();
  await page.locator('uc-settings textarea#cb').fill('[{"label": "Echo", "cmd": "echo button"}]');
  await page.getByRole('button', { name: 'Save' }).click();
  await page.getByRole('tab', { name: 'Commands' }).click();
  await page.getByRole('button', { name: 'Echo', exact: true }).click();
  await expect(page.locator('uc-commands .out')).toContainText('button');
  expect(errors).toEqual([]);
});
