// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * The logger panel (plan 03 §2): a WebviewPanel with `<uc-log>`, kept alive while hidden. Bytes go
 * webview ⇄ postMessage ⇄ the extension host's TCP connection to a hub.
 */

import * as vscode from 'vscode';
import { randomBytes } from 'node:crypto';
import { HubConnection } from './connection.ts';
import type { ConnSummary, FromWebview, LinkState, ToWebview, WebviewInit } from './messages.ts';

/** Where a link goes: a hub address, found when the webview opens the link (a private hub may
 * have to start first). */
export interface Target {
  description: string;
  /** Called on open; may start a hub. Throws with a message for the user. */
  resolve(progress: (detail: string) => void): Promise<{ host: string; port: number }>;
  /** Called when the link closes. */
  released?(): void;
}

export interface PanelHost {
  init(): WebviewInit;
  /** A `call` from the webview (services and pickers). */
  call(op: string, args: unknown[]): Promise<unknown>;
  onMessage(m: FromWebview): void;
  onState(conn: ConnSummary): void;
  onDispose(): void;
}

export class LoggerPanel {
  static readonly viewType = 'uclog.logger';
  readonly panel: vscode.WebviewPanel;
  private ready: Promise<void>;
  private setReady!: () => void;
  private nextId = 1;
  private targets = new Map<number, Target>();
  private conns = new Map<number, HubConnection>();
  conn: ConnSummary = { link: 'idle', liveness: null };
  private disposed = false;

  constructor(
    private ctx: vscode.ExtensionContext,
    private host: PanelHost,
    column: vscode.ViewColumn = vscode.ViewColumn.Active,
  ) {
    const dist = vscode.Uri.joinPath(ctx.extensionUri, 'dist', 'webview');
    this.panel = vscode.window.createWebviewPanel(LoggerPanel.viewType, 'ucLog', column, {
      enableScripts: true,
      retainContextWhenHidden: true,
      localResourceRoots: [dist],
    });
    this.panel.iconPath = vscode.Uri.joinPath(ctx.extensionUri, 'media', 'uclog.svg');
    this.ready = new Promise((r) => (this.setReady = r));
    this.panel.webview.html = this.html(dist);
    this.panel.webview.onDidReceiveMessage((m: FromWebview) => this.onMessage(m));
    this.panel.onDidDispose(() => {
      this.disposed = true;
      // Targets too: a link whose private hub is still starting has no connection yet.
      for (const id of new Set([...this.conns.keys(), ...this.targets.keys()])) this.closeLink(id);
      this.host.onDispose();
    });
  }

  reveal() {
    this.panel.reveal(undefined, false);
  }

  whenReady() {
    return this.ready;
  }

  post(m: ToWebview) {
    if (!this.disposed) void this.panel.webview.postMessage(m);
  }

  /** Connect the logger to `t` (the webview disconnects the previous link first). */
  async connect(t: Target) {
    await this.ready;
    const id = this.nextId++;
    this.targets.set(id, t);
    this.post({ t: 'connect', id, description: t.description });
  }

  disconnect() {
    this.post({ t: 'disconnect' });
  }

  private linkState(id: number, state: LinkState, detail?: string) {
    this.post({ t: 'link', id, state, detail });
  }

  private async openLink(id: number) {
    const t = this.targets.get(id);
    if (!t) return this.linkState(id, 'error', 'unknown link');
    let addr: { host: string; port: number };
    try {
      addr = await t.resolve((d) => this.linkState(id, 'connecting', d));
    } catch (e) {
      this.targets.delete(id);
      t.released?.();
      return this.linkState(id, 'error', (e as Error).message);
    }
    if (this.disposed || !this.targets.has(id)) {
      // Closed while the hub started: release again, as the hub may not have existed at close.
      t.released?.();
      return;
    }
    // A repeated `open` for the same link replaces its connection.
    this.conns.get(id)?.close();
    let c: HubConnection | undefined;
    c = new HubConnection(
      addr.host,
      addr.port,
      (s, detail) => {
        if (c && this.conns.get(id) !== c) return; // replaced
        this.linkState(id, s, detail);
        if (s !== 'connecting' && s !== 'approval' && s !== 'open') this.closeLink(id);
      },
      (b) => this.post({ t: 'rx', id, data: b }),
    );
    this.conns.set(id, c);
  }

  private closeLink(id: number) {
    this.conns.get(id)?.close();
    this.conns.delete(id);
    const t = this.targets.get(id);
    this.targets.delete(id);
    t?.released?.();
  }

  private onMessage(m: FromWebview) {
    switch (m.t) {
      case 'ready':
        this.setReady();
        break;
      case 'open':
        void this.openLink(m.id);
        return;
      case 'tx':
        this.conns.get(m.id)?.write(m.data instanceof Uint8Array ? m.data : new Uint8Array(Object.values(m.data)));
        return;
      case 'close':
        return this.closeLink(m.id);
      case 'state':
        this.conn = m.conn;
        break;
      case 'call':
        this.host.call(m.op, m.args).then(
          (value) => this.post({ t: 'reply', id: m.id, ok: true, value }),
          (e: Error & { errno?: string }) => this.post({ t: 'reply', id: m.id, ok: false, error: e.message, errno: e.errno }),
        );
        return;
    }
    this.host.onMessage(m);
    if (m.t === 'state') this.host.onState(m.conn);
  }

  private html(dist: vscode.Uri): string {
    const w = this.panel.webview;
    const nonce = randomBytes(16).toString('base64');
    const script = w.asWebviewUri(vscode.Uri.joinPath(dist, 'webview.js'));
    const init = JSON.stringify(this.host.init()).replace(/</g, '\\u003c');
    // Inline workers are blob: URLs; uPlot's chunk and the script come from dist/webview.
    const csp = [
      "default-src 'none'",
      `script-src 'nonce-${nonce}' ${w.cspSource}`,
      `style-src ${w.cspSource} 'unsafe-inline'`,
      `img-src ${w.cspSource} data: blob:`,
      `font-src ${w.cspSource}`,
      'worker-src blob:',
    ].join('; ');
    return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>ucLog</title>
<style>
  html, body { height: 100%; margin: 0; padding: 0; overflow: hidden; background: var(--vscode-editor-background); }
  uc-log {
    height: 100%;
    border: none;
    --uc-radius: 0;
    --uc-font: var(--vscode-font-family);
    --uc-font-size: var(--vscode-font-size);
    --uc-mono: var(--vscode-editor-font-family);
    --uc-bg: var(--vscode-editor-background);
    --uc-panel: var(--vscode-editor-background);
    --uc-fg: var(--vscode-foreground);
    --uc-muted: var(--vscode-descriptionForeground);
    --uc-border: var(--vscode-panel-border, var(--vscode-widget-border, rgba(128,128,128,.35)));
    --uc-accent: var(--vscode-button-background);
    --uc-accent-fg: var(--vscode-button-foreground);
    --uc-button-bg: var(--vscode-button-secondaryBackground, transparent);
    --uc-button-hover: var(--vscode-button-secondaryHoverBackground, var(--vscode-list-hoverBackground));
    --uc-input-bg: var(--vscode-input-background);
    --uc-selection: var(--vscode-list-inactiveSelectionBackground, var(--vscode-editor-selectionBackground));
    --uc-term-bg: var(--vscode-terminal-background, var(--vscode-editor-background));
    --uc-error: var(--vscode-errorForeground);
    --uc-warn: var(--vscode-editorWarning-foreground);
    --uc-ok: var(--vscode-testing-iconPassed, #3fb950);
  }
</style>
<script type="application/json" id="uclog-init">${init}</script>
</head>
<body>
<script type="module" nonce="${nonce}" src="${script}"></script>
</body>
</html>`;
  }
}
