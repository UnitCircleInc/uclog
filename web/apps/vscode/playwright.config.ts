import { defineConfig } from '@playwright/test';

// The extension in a real VS Code (Playwright's Electron driver). Needs a VS Code install
// (UCLOG_VSCODE, or the default macOS path) and bin/ filled (node scripts/package.mjs --local
// --no-package) and dist/ built.
export default defineConfig({
  testDir: 'e2e',
  timeout: 120_000,
  workers: 1,
  reporter: [['list']],
  use: { trace: 'retain-on-failure' },
});
