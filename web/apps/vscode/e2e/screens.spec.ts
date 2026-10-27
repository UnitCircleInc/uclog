// Screenshots for the docs (website/build.py screenshots): the logger panel in VS Code with the
// simulated device, its logs beside the source they came from. From the VSIX (UCLOG_VSIX), so the
// window is not an "Extension Development Host".
//   UCLOG_SHOTS=dir UCLOG_VSIX=apps/vscode/uclog-….vsix npx playwright test -c apps/vscode/playwright.config.ts screens

import { expect, test } from '@playwright/test';
import { cpSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { Code, haveCode, repo } from './util.ts';

const DIR = process.env.UCLOG_SHOTS;

test('screens', async () => {
  test.skip(!haveCode || !DIR, 'set UCLOG_SHOTS, install VS Code');
  test.setTimeout(120_000);
  const code = new Code({
    'uclog.keepHubRunning': false,
    'workbench.colorTheme': 'Default Dark Modern',
    'workbench.secondarySideBar.defaultVisibility': 'hidden',
    'chat.disableAIFeatures': true,
    'workbench.tips.enabled': false,
    'extensions.ignoreRecommendations': true,
  });
  mkdirSync(join(code.ws, 'build'));
  cpSync(join(repo, 'spec/vectors/sample.logdata'), join(code.ws, 'build', `${'5a'.repeat(64)}.logdata`));
  mkdirSync(join(code.ws, 'src'));
  // The simulated device's logs come from src/main.c lines 10, 11 and 20.
  writeFileSync(
    join(code.ws, 'src/main.c'),
    [
      '#include "board.h"',
      '#include "log.h"',
      '',
      'int main(void) {',
      '  log_pre_init();',
      '  board_init();',
      '  log_init(BOARD_NAME, "app");',
      '  for (unsigned n = 0;; n++) {',
      '    if (n % 3 == 0) {',
      '      LOG_INFO("Hello World! loopback #%u", n);',
      '      LOG_WARN("temp=%d mV=%u flags=0x%08x", t, mv, flags);',
      '    }',
      '    log_poll();',
      '    board_sleep_ms(100);',
      '    if (n % 3 != 1) {',
      '      continue;',
      '    }',
      '    // Once in three passes: a log with no arguments, sent as',
      '    // the format string\'s address alone (4 bytes).',
      '    LOG_INFO("no args");',
      '  }',
      '}',
      '',
    ].join('\n'),
  );
  await code.launch();
  try {
    const page = code.page;
    await page.setViewportSize({ width: 1400, height: 860 });
    await code.command('View: Close Primary Side Bar');
    await code.command('ucLog: Connect…');
    await page.locator('.quick-input-list .monaco-list-row', { hasText: 'Simulated device' }).click();
    const f = await code.logger();
    await expect(f.locator('uc-log-view .r').first()).toContainText('Hello World!', { timeout: 30_000 });
    // A log's source: click-through opens src/main.c beside the panel.
    await f.locator('uc-log-view .r .src', { hasText: 'src/main.c:10' }).first().click();
    await expect(page.locator('.tabs-container .tab', { hasText: 'main.c' })).toBeVisible({ timeout: 15_000 });
    await code.command('Notifications: Clear All Notifications');
    await page.waitForTimeout(2000);
    await page.screenshot({ path: join(DIR!, 'vscode-logs.png') });
  } finally {
    await code.close();
  }
});
