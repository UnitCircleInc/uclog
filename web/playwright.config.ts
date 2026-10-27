import { defineConfig } from '@playwright/test';

// Chrome (Web Serial/HID/Bluetooth are Chromium-only). Uses the installed Google Chrome; set
// UCLOG_E2E_CHANNEL=chromium to use Playwright's own build instead.
export default defineConfig({
  testDir: 'e2e',
  globalSetup: './e2e/global-setup.ts',
  timeout: 60_000,
  fullyParallel: false,
  workers: 1,
  reporter: [['list']],
  use: {
    baseURL: 'http://localhost:5199',
    channel: process.env.UCLOG_E2E_CHANNEL ?? 'chrome',
    viewport: { width: 1280, height: 820 },
    trace: 'retain-on-failure',
  },
  webServer: {
    command: 'npx vite --port 5199 --strictPort',
    cwd: 'apps/demo',
    url: 'http://localhost:5199',
    reuseExistingServer: !process.env.CI,
  },
});
