// The extension in VS Code against the simulated device (private hubs) and an external hub.

import { expect, test } from '@playwright/test';
import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { cpSync, mkdirSync, writeFileSync } from 'node:fs';
import * as net from 'node:net';
import { join } from 'node:path';
import { Code, extRoot, haveCode, HUB, repo } from './util.ts';

const HASH = '5a'.repeat(64);

test.skip(!haveCode, 'VS Code is not installed (set UCLOG_VSCODE)');
// One VS Code for the whole file; each test builds on the previous one's state.
test.describe.configure({ mode: 'serial' });

let code: Code;
let page: import('@playwright/test').Page;
const command = (t: string) => code.command(t);
const logger = () => code.logger();

/** Private hubs for the simulated device (this test's only). */
function loopbackHubs(): number {
  try {
    return execFileSync('pgrep', ['-f', `${extRoot}/bin/.*uclog-hub -d loopback`]).toString().trim().split('\n').filter(Boolean).length;
  } catch {
    return 0;
  }
}

test.beforeAll(async () => {
  code = new Code({ 'uclog.keepHubRunning': false, 'uclog.terminal.onConnect': ['help'] });
  const ws = code.ws;
  // The simulated device's app_hash is 5a…5a: its logdata is found in the workspace by name.
  mkdirSync(join(ws, 'build', 'zephyr'), { recursive: true });
  cpSync(join(repo, 'spec/vectors/sample.logdata'), join(ws, 'build', `${HASH}.logdata`));
  writeFileSync(join(ws, 'build', 'zephyr', 'zephyr.signed.bin'), new Uint8Array(4096).fill(0xa5));
  mkdirSync(join(ws, 'src'));
  writeFileSync(join(ws, 'src/main.c'), Array.from({ length: 20 }, (_, i) => `// line ${i + 1}`).join('\n'));
  await code.launch();
  page = code.page;
});

test.afterAll(() => code?.close());

test('simulated device through a private hub', async () => {
  await command('ucLog: Connect…');
  await page.locator('.quick-input-list .monaco-list-row', { hasText: 'Simulated device' }).click();
  const f = await logger();
  // Logs decode with the workspace's <app_hash>.logdata (sibling lookup in the extension host).
  await expect(f.locator('uc-log footer')).toContainText('logdata: sibling', { timeout: 30_000 });
  await expect(f.locator('uc-log-view .r').first()).toContainText('Hello World!', { timeout: 30_000 });
  await expect(page.locator('#workbench\\.parts\\.statusbar')).toContainText('ucLog: Simulated device');
  // The on-connect line reached the device terminal.
  await f.locator('uc-log nav button', { hasText: 'Terminal' }).click();
  await expect(f.locator('uc-terminal .xterm-rows')).toContainText('loopback commands', { timeout: 15_000 });
  await page.screenshot({ path: join('apps/vscode/test-results', 'vscode-terminal.png') });
});

test('click-through opens the source', async () => {
  const f = await logger();
  await f.locator('uc-log nav button', { hasText: 'Logs' }).click();
  await f.locator('uc-log-view .r .src', { hasText: 'src/main.c:10' }).first().click();
  await expect(page.locator('.tabs-container .tab', { hasText: 'main.c' })).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.editor-instance .view-lines')).toContainText('// line 10');
});

test('semihost console output and time through the hub', async () => {
  const f = await logger();
  await f.locator('uc-log nav button', { hasText: 'Terminal' }).click();
  const input = f.locator('uc-terminal .input input');
  await input.fill('sh');
  await input.press('Enter');
  // TIMEOFDAY is answered by the hub; the WRITE to stdout is forwarded to us.
  await expect(f.locator('uc-terminal .xterm-rows')).toContainText('sh: TIMEOFDAY sec=Some(', { timeout: 15_000 });
  await expect(f.locator('uc-terminal .xterm-rows')).toContainText('hello from loopback');
  await expect(f.locator('uc-terminal .xterm-rows')).toContainText('sh: WRITE r=Some(20) errno=Some(0)');
});

test('device terminal (pseudoterminal)', async () => {
  await command('ucLog: Open Device Terminal');
  const term = page.locator('.terminal-wrapper.active .xterm-rows, .terminal .xterm-rows').last();
  await expect(term).toContainText('ucLog device terminal', { timeout: 15_000 });
  await term.click(); // the keys go to the focused element: not always the new terminal yet
  await page.keyboard.type('echo from vscode');
  await page.keyboard.press('Enter');
  await expect(term).toContainText('from vscode', { timeout: 15_000 });
  await page.screenshot({ path: join('apps/vscode/test-results', 'vscode-pty.png') });
});

test('disconnect stops the private hub (keepHubRunning off)', async () => {
  expect(loopbackHubs()).toBe(1);
  await command('ucLog: Disconnect');
  const f = await logger();
  await expect(f.locator('uc-log footer')).toContainText('disconnected', { timeout: 15_000 });
  await expect.poll(loopbackHubs, { timeout: 10_000 }).toBe(0);
});

test('a hub kept running buffers while the logger is closed', async () => {
  code.settings({ 'uclog.keepHubRunning': true });
  await page.waitForTimeout(1000);
  await command('ucLog: Connect…');
  await page.locator('.quick-input-list .monaco-list-row', { hasText: 'Simulated device' }).click();
  let f = await logger();
  await expect(f.locator('uc-log footer')).toContainText('simulated device', { timeout: 30_000 });
  // Close the logger: the hub stays.
  const tab = page.locator('.tabs-container .tab', { hasText: 'ucLog' });
  await tab.hover();
  await tab.getByRole('button', { name: /^Close/ }).click();
  await page.waitForTimeout(1500);
  expect(loopbackHubs()).toBe(1);
  // Reopening connects again; the backlog fills the log at once.
  await command('ucLog: Open Logger');
  f = await logger();
  await expect(f.locator('uc-log-view .r').first()).toContainText('Hello World!', { timeout: 30_000 });
  await command('ucLog: Disconnect');
});

test('Devices view: click the simulated device', async () => {
  await page.locator('.activitybar a[aria-label^="ucLog"]').first().click();
  const item = page.locator('.monaco-list-row', { hasText: 'Simulated device' });
  await expect(item).toBeVisible({ timeout: 30_000 });
  await item.click();
  const f = await logger();
  await expect(f.locator('uc-log footer')).toContainText('simulated device', { timeout: 30_000 });
  await expect(page.locator('#workbench\\.parts\\.statusbar')).toContainText('ucLog: Simulated device');
  await expect(item).toContainText('connected');
});

test('FW update: a workspace image, then the device says it has no FW update', async () => {
  await command('ucLog: Flash Firmware…');
  await page.locator('.quick-input-list .monaco-list-row', { hasText: 'zephyr.signed.bin' }).click();
  const f = await logger();
  await expect(f.locator('uc-fw-update')).toContainText('build/zephyr/zephyr.signed.bin, 4,096 bytes');
  // The simulated device has no port 59 or 3.
  await expect(f.locator('uc-fw-update')).toContainText('does not support FW update', { timeout: 20_000 });
  await command('ucLog: Disconnect');
});

/** A hub outside VS Code (as on another machine). */
async function externalHub(): Promise<{ proc: ChildProcess; port: number }> {
  const exe = HUB!;
  const proc = spawn(exe, ['-d', 'loopback:rate=5', '--bind', '127.0.0.1', '--port', '0', '--no-mdns', '--no-registry', '--json-ready', '--exit-on-stdin-eof']);
  const port = await new Promise<number>((resolve, reject) => {
    proc.stdout!.once('data', (b: Buffer) => resolve(JSON.parse(b.toString()).port));
    proc.once('exit', () => reject(new Error('hub exited')));
  });
  return { proc, port };
}

test('a hub by address: busy, then connected', async () => {
  const { proc, port } = await externalHub();
  try {
    // Another client holds the hub.
    const other = net.connect(port, '127.0.0.1');
    await new Promise((r) => other.once('data', r));
    await command('ucLog: Enter Hub Address…');
    await page.locator('.quick-input-widget input').fill(`127.0.0.1:${port}`);
    await page.keyboard.press('Enter');
    const f = await logger();
    await expect(f.locator('uc-log footer')).toContainText('hub busy', { timeout: 15_000 });
    await expect(page.locator('#workbench\\.parts\\.statusbar')).toContainText('ucLog: hub busy');
    other.destroy();
    await page.waitForTimeout(500);
    // Recent hubs are offered in Connect….
    await command('ucLog: Connect…');
    await page.locator('.quick-input-list .monaco-list-row', { hasText: `tcp://127.0.0.1:${port}` }).click();
    await expect(f.locator('uc-log-view .r').first()).toContainText('Hello World!', { timeout: 30_000 });
    await expect(page.locator('#workbench\\.parts\\.statusbar')).toContainText(`ucLog: tcp://127.0.0.1:${port}`);
    await page.screenshot({ path: join('apps/vscode/test-results', 'vscode-hub.png') });
    await command('ucLog: Disconnect');
  } finally {
    proc.kill();
  }
});
