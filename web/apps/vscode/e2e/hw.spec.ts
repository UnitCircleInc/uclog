// Hardware through VS Code (a private hub on the device):
//   UCLOG_TEST_DEVICE=serial:/dev/cu.usbmodem… npx playwright test -c apps/vscode/playwright.config.ts hw
// The workspace holds the NUCLEO build's <app_hash>.logdata files, found by the extension host;
// then `test` and `burst` must decode cleanly.

import { expect, test } from '@playwright/test';
import { cpSync, existsSync, mkdirSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { Code, decodeState, haveCode, repo } from './util.ts';

const DEV = process.env.UCLOG_TEST_DEVICE;
const BUILD = join(repo, 'firmware/nucleo-l452re-p/build');

test('real device through a private hub: workspace logdata, test, burst', async () => {
  test.skip(!haveCode || !DEV || !existsSync(BUILD), 'set UCLOG_TEST_DEVICE, build the NUCLEO firmware, install VS Code');
  test.setTimeout(180_000);
  const code = new Code({ 'uclog.keepHubRunning': false });
  mkdirSync(join(code.ws, 'build'));
  for (const f of readdirSync(BUILD).filter((f) => /^[0-9a-f]{128}\.logdata$/.test(f))) cpSync(join(BUILD, f), join(code.ws, 'build', f));
  await code.launch();
  try {
    await code.command('ucLog: Connect…');
    await code.pick(DEV!);
    const f = await code.logger();
    await expect(f.locator('uc-log footer')).toContainText('logdata: sibling', { timeout: 30_000 });
    await f.locator('uc-log nav button', { hasText: 'Terminal' }).click();
    const input = f.locator('uc-terminal .input input');
    await input.fill('test');
    await input.press('Enter');
    await expect.poll(async () => (await decodeState(f))?.received ?? 0, { timeout: 20_000 }).toBeGreaterThan(20);
    const before = (await decodeState(f))!.received;
    await input.fill('burst 5000');
    await input.press('Enter');
    await expect(f.locator('uc-terminal .xterm-rows')).toContainText('burst of 5000 logs', { timeout: 30_000 });
    await expect.poll(async () => (await decodeState(f))!.received - before, { timeout: 30_000 }).toBeGreaterThanOrEqual(5000);
    const st = (await decodeState(f))!;
    console.log(`received ${st.received}, error records ${JSON.stringify(st.errorCounts)}`);
    expect(st.errorCounts).toEqual({});
    await f.locator('uc-log nav button', { hasText: 'Device' }).click();
    await expect(f.locator('uc-device-info')).toContainText('app_hash');
    await code.page.screenshot({ path: join(code.ws, '..', 'hw.png') });
    await code.page.screenshot({ path: 'apps/vscode/test-results/vscode-hw.png' });
    await code.command('ucLog: Disconnect');
  } finally {
    await code.close();
  }
});
