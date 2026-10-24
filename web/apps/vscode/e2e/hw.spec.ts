// Hardware through VS Code (a private hub on the device):
//   UCLOG_TEST_DEVICE=serial:/dev/cu.usbmodem… npx playwright test -c apps/vscode/playwright.config.ts hw
//   UCLOG_TEST_FIRMWARE=nucleo-c071rb UCLOG_TEST_DEVICE=hid:<serial> …   (the C071's USB links)
//   UCLOG_TEST_FIRMWARE=stm32wba55g-dk1 UCLOG_TEST_DEVICE="ble:<advertised name>" …   (BLE)
// The workspace holds the firmware build's <app_hash>.logdata files, found by the extension host;
// then `test` and `burst` must decode cleanly, and `sh` must create its files in the workspace
// (semihost through the extension).

import { expect, test } from '@playwright/test';
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Code, decodeState, haveCode, repo } from './util.ts';

const DEV = process.env.UCLOG_TEST_DEVICE;
const BUILD = join(repo, 'firmware', process.env.UCLOG_TEST_FIRMWARE ?? 'nucleo-l452re-p', 'build');

test('real device through a private hub: workspace logdata, test, burst', async () => {
  test.skip(!haveCode || !DEV || !existsSync(BUILD), 'set UCLOG_TEST_DEVICE, build the NUCLEO firmware, install VS Code');
  test.setTimeout(180_000);
  const code = new Code({ 'uclog.keepHubRunning': false });
  mkdirSync(join(code.ws, 'build'));
  for (const f of readdirSync(BUILD).filter((f) => /^[0-9a-f]{128}\.logdata$/.test(f))) cpSync(join(BUILD, f), join(code.ws, 'build', f));
  await code.launch();
  try {
    await code.command('ucLog: Connect…');
    if (DEV!.startsWith('ble:')) {
      await code.pick('Scan for Bluetooth devices'); // then listed by name
      await code.pick(DEV!.slice(4));
    } else {
      await code.pick(DEV!);
    }
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
    let st = (await decodeState(f))!;
    console.log(`received ${st.received}, error records ${JSON.stringify(st.errorCounts)}`);
    expect(st.errorCounts).toEqual({});
    // Semihost: the device writes, reads back and checks files in the workspace folder.
    await input.fill('sh');
    await input.press('Enter');
    await expect(f.locator('uc-terminal .xterm-rows')).toContainText('semihost selftest done', { timeout: 30_000 });
    await expect.poll(() => existsSync(join(code.ws, 'uclog-big.bin')), { timeout: 10_000 }).toBe(true);
    expect(readFileSync(join(code.ws, 'uclog-test.txt'), 'utf8')).toMatch(/^written by nucleo at \d+\n$/);
    st = (await decodeState(f))!;
    expect(st.errorCounts).toEqual({});
    await f.locator('uc-log nav button', { hasText: 'Logs' }).click();
    await f.locator('uc-log-view input[type=search]').fill('expected|round trip|semihost selftest done');
    await expect(f.locator('uc-log-view')).toContainText('semihost selftest done');
    await expect(f.locator('uc-log-view')).not.toContainText(', want ');
    await f.locator('uc-log nav button', { hasText: 'Device' }).click();
    await expect(f.locator('uc-device-info')).toContainText('app_hash');
    await code.page.screenshot({ path: join(code.ws, '..', 'hw.png') });
    await code.page.screenshot({ path: 'apps/vscode/test-results/vscode-hw.png' });
    await code.command('ucLog: Disconnect');
  } finally {
    await code.close();
  }
});
