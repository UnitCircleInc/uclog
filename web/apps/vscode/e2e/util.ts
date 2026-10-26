// Launching VS Code with the extension under development, and driving it.

import { _electron, type ElectronApplication, type Frame, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const ext = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const repo = resolve(ext, '../../..');
export const CODE =
  process.env.UCLOG_VSCODE ??
  (process.platform === 'darwin' ? '/Applications/Visual Studio Code.app/Contents/MacOS/Code' : process.platform === 'win32' ? '' : '/usr/share/code/code');
export const haveCode = !!CODE && existsSync(CODE);
/** A uclog-hub build (for hubs outside VS Code). */
export const HUB = ['release', 'debug'].map((p) => join(repo, `native/target/${p}/uclog-hub${process.platform === 'win32' ? '.exe' : ''}`)).find((p) => existsSync(p));
/** The `code` command line (to install a VSIX). */
const CLI = process.env.UCLOG_VSCODE_CLI ?? (process.platform === 'darwin' ? resolve(CODE, '../../Resources/app/bin/code') : 'code');
/** Test an installed VSIX instead of the source folder. */
export const VSIX = process.env.UCLOG_VSIX ? resolve(process.env.UCLOG_VSIX) : null;
/** Where the extension's files are (the source folder, or the installed copy). */
export let extRoot = ext;

const SETTINGS = {
  'workbench.startupEditor': 'none',
  'security.workspace.trust.enabled': false,
  'window.restoreWindows': 'none',
  'update.mode': 'none',
  'telemetry.telemetryLevel': 'off',
  'workbench.tips.enabled': false,
  'terminal.integrated.gpuAcceleration': 'off',
};

export class Code {
  app!: ElectronApplication;
  page!: Page;
  readonly tmp = mkdtempSync(join(tmpdir(), 'uclog-vscode-'));
  readonly ws = join(this.tmp, 'workspace');
  private settingsFile = join(this.tmp, 'user', 'User', 'settings.json');

  constructor(private base: Record<string, unknown> = {}) {
    mkdirSync(this.ws, { recursive: true });
    mkdirSync(dirname(this.settingsFile), { recursive: true });
    this.settings({});
  }

  settings(extra: Record<string, unknown>) {
    writeFileSync(this.settingsFile, JSON.stringify({ ...SETTINGS, ...this.base, ...extra }));
  }

  async launch() {
    const exts = join(this.tmp, 'extensions');
    if (VSIX) {
      execFileSync(CLI, ['--extensions-dir', exts, '--user-data-dir', join(this.tmp, 'user'), '--install-extension', VSIX], { stdio: 'inherit' });
      extRoot = join(exts, `unitcircle.uclog-${JSON.parse(readFileSync(join(ext, 'package.json'), 'utf8')).version}`);
    }
    this.app = await _electron.launch({
      executablePath: CODE,
      args: [
        ...(VSIX ? [] : [`--extensionDevelopmentPath=${ext}`, '--disable-extensions']),
        // CI runners have no setuid sandbox helper.
        ...(process.env.CI && process.platform === 'linux' ? ['--no-sandbox'] : []),
        `--user-data-dir=${join(this.tmp, 'user')}`,
        `--extensions-dir=${join(this.tmp, 'extensions')}`,
        '--skip-welcome',
        '--skip-release-notes',
        '--disable-workspace-trust',
        '--new-window',
        this.ws,
      ],
      // Not the shared logdata cache (~/.cache/uclog), which the firmware builds fill.
      env: { ...process.env, UCLOG_CACHE_DIR: join(this.tmp, 'cache') },
    });
    this.page = await this.app.firstWindow();
    await this.page.waitForSelector('.monaco-workbench', { timeout: 60_000 });
  }

  async close() {
    await this.app?.close();
    if (process.env.UCLOG_KEEP) console.log(`kept ${this.tmp}`);
    else rmSync(this.tmp, { recursive: true, force: true });
  }

  /** Run a command from the palette by its title. */
  async command(title: string) {
    await this.page.keyboard.press(process.platform === 'darwin' ? 'Meta+Shift+P' : 'Control+Shift+P');
    await this.page.locator('.quick-input-widget input').fill(`>${title}`);
    await this.page.locator('.quick-input-list .monaco-list-row', { hasText: title }).first().click();
  }

  pick(text: string) {
    return this.page.locator('.quick-input-list .monaco-list-row', { hasText: text }).first().click();
  }

  get statusBar() {
    return this.page.locator('#workbench\\.parts\\.statusbar');
  }

  /** The frame with <uc-log> (VS Code nests webviews in two iframes). */
  async logger(): Promise<Frame> {
    for (let i = 0; i < 100; i++) {
      for (const f of this.page.frames()) {
        if (await f.locator('uc-log').count().catch(() => 0)) return f;
      }
      await this.page.waitForTimeout(200);
    }
    throw new Error('no logger webview');
  }
}

/** The logger's decode state (counts and error records). */
export function decodeState(f: Frame) {
  return f.evaluate(() => (document.querySelector('uc-log') as unknown as { client: { decodeState: { received: number; errorCounts: Record<string, number> } | null } }).client.decodeState);
}
