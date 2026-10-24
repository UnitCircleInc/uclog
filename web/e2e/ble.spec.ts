// Hardware over Web Bluetooth, no hub: the browser's Bluetooth chooser is answered through Chrome's
// DevTools DeviceAccess domain (it covers Bluetooth only; Web Serial and WebHID choosers cannot be
// automated).
//   UCLOG_TEST_BLE="ucLog test STM32WBA55G-DK1" UCLOG_TEST_ELF=../firmware/stm32wba55g-dk1/build/app.elf \
//   npx playwright test ble
// The ELF's .logdata_cbor is loaded in the page, then `test`, `burst` and a host → device frame of
// the maximum size (written in 512-byte long writes) must go through cleanly.

import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import { decodeState, watchErrors } from './util.ts';

const NAME = process.env.UCLOG_TEST_BLE;
const ELF = process.env.UCLOG_TEST_ELF;

test('real device over Web Bluetooth: device info, ELF logdata, test, burst, echo', async ({ page }) => {
  test.skip(!NAME || !ELF, 'set UCLOG_TEST_BLE (the advertised name) and UCLOG_TEST_ELF');
  test.setTimeout(120_000);
  const errors = watchErrors(page);
  await page.goto('/');
  const elf = [...readFileSync(ELF!)];
  await page.evaluate((b) => (window as unknown as { ucLog: { loadLogdata(f: object): Promise<unknown> } }).ucLog.loadLogdata({ name: 'app.elf', data: new Uint8Array(b) }), elf);

  // Answer the chooser with the device once the scan has found it.
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('DeviceAccess.enable');
  let chosen = false;
  cdp.on('DeviceAccess.deviceRequestPrompted', (e: { id: string; devices: { id: string; name: string }[] }) => {
    const d = e.devices.find((x) => x.name === NAME);
    if (d && !chosen) {
      chosen = true;
      void cdp.send('DeviceAccess.selectPrompt', { id: e.id, deviceId: d.id });
    }
  });
  await page.getByRole('button', { name: 'Connect…' }).click();
  await page.getByRole('button', { name: 'Bluetooth…' }).click();
  await expect(page.locator('footer')).toContainText('logdata: manual app.elf', { timeout: 30_000 });

  await page.getByRole('tab', { name: 'Terminal' }).click();
  const input = page.getByLabel('Terminal command');
  await input.fill('test');
  await input.press('Enter');
  await expect.poll(async () => (await decodeState(page))?.received ?? 0, { timeout: 20_000 }).toBeGreaterThan(20);
  const before = (await decodeState(page))!.received;
  await input.fill('burst 5000');
  await input.press('Enter');
  await expect(page.locator('.xterm-rows')).toContainText('burst of 5000 logs', { timeout: 30_000 });
  await expect.poll(async () => (await decodeState(page))!.received - before, { timeout: 30_000 }).toBeGreaterThanOrEqual(5000);

  // Host → device: a 1438-byte command line (one maximum frame), which the device answers.
  const long = 'x'.repeat(1400);
  await input.fill(`echo-check ${long}`);
  await input.press('Enter');
  await expect(page.locator('.xterm-rows')).toContainText('unknown command "echo-check"', { timeout: 15_000 });

  const st = (await decodeState(page))!;
  console.log(`received ${st.received}, error records ${JSON.stringify(st.errorCounts)}`);
  expect(st.errorCounts).toEqual({});
  await page.getByRole('tab', { name: 'Device' }).click();
  await expect(page.locator('uc-device-info')).toContainText('app_hash');
  expect(errors).toEqual([]);
});
