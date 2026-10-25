// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * `<uc-log>`: the embeddable ucLog client (plan 02 §7).
 *
 * Attributes: `logdata-domain`, `logdata-url`, `hub-url`, `panels`, `layout` (`tabs|grid`),
 * `theme` (`auto|light|dark`), `storage-key`, `worker-base`, `emit-logs`.
 * Property: `logdataToken`. Methods: `connect(spec)`, `disconnect()`, `send(port, bytes)`,
 * `loadLogdata(file)`, `ready()`. Events: `uc-ready`, `uc-connect`, `uc-disconnect`, `uc-device-info`,
 * `uc-log` (with `emit-logs`), `uc-error`. Styling: `--uc-*` custom properties and `::part()`.
 */

import { css, html, LitElement, nothing } from 'lit';
import { Client, workersAt, type ConnectionState, type LinkSpec, type WorkerFactories } from '@uclog/workers';
import { base } from './styles.ts';
import { SettingsStore } from './settings.ts';
import { download, readFile, setSaveHook, stamp, zip } from './util.ts';
import type { UcHost } from './host.ts';
import './log-view.ts';
import './terminal.ts';
import './commands.ts';
import './plots.ts';
import './device-info.ts';
import './fw-update.ts';
import './device-picker.ts';
import './settings-dialog.ts';
import type { UcDevicePicker } from './device-picker.ts';
import type { UcSettings } from './settings-dialog.ts';
import type { UcTerminal } from './terminal.ts';
import type { UcPlots } from './plots.ts';
import type { UcFwUpdate } from './fw-update.ts';

export const PANELS = ['log', 'terminal', 'commands', 'plots', 'device', 'fw-update'] as const;
export type Panel = (typeof PANELS)[number];
const TITLES: Record<Panel, string> = {
  log: 'Logs',
  terminal: 'Terminal',
  commands: 'Commands',
  plots: 'Plots',
  device: 'Device',
  'fw-update': 'FW update',
};

const LIGHT = css`
  --uc-bg: #f5f6f8;
  --uc-panel: #ffffff;
  --uc-fg: #1d1f24;
  --uc-muted: #6b7280;
  --uc-border: #d9dce1;
  --uc-accent: #2563eb;
  --uc-accent-fg: #ffffff;
  --uc-button-bg: #ffffff;
  --uc-button-hover: #eef1f5;
  --uc-input-bg: #ffffff;
  --uc-selection: #dbe7ff;
  --uc-term-bg: #fbfbfc;
  --uc-error: #c62828;
  --uc-warn: #b26a00;
  --uc-ok: #2e7d32;
`;
const DARK = css`
  --uc-bg: #101216;
  --uc-panel: #16181d;
  --uc-fg: #d7dae0;
  --uc-muted: #9097a3;
  --uc-border: #2c313a;
  --uc-accent: #4f8cff;
  --uc-accent-fg: #ffffff;
  --uc-button-bg: #1d2027;
  --uc-button-hover: #262a33;
  --uc-input-bg: #12141a;
  --uc-selection: #24345a;
  --uc-term-bg: #16181d;
  --uc-error: #ff6b6b;
  --uc-warn: #f0b429;
  --uc-ok: #5cc06a;
`;

export class UcLog extends LitElement {
  /** Worker factories when `worker-base` is not set: the element bundle sets inlined ones, an app
   * built with Vite can use `@uclog/workers/default-workers`. */
  static workers: WorkerFactories | null = null;
  /** Set by an embedding host that provides its own pickers, settings and services (VS Code). */
  static host: UcHost | null = null;

  static properties = {
    logdataDomain: { type: String, attribute: 'logdata-domain' },
    logdataUrl: { type: String, attribute: 'logdata-url' },
    hubUrl: { type: String, attribute: 'hub-url' },
    panels: { type: String },
    layout: { type: String },
    theme: { type: String },
    storageKey: { type: String, attribute: 'storage-key' },
    workerBase: { type: String, attribute: 'worker-base' },
    emitLogs: { type: Boolean, attribute: 'emit-logs' },
    active: { state: true },
    conn: { state: true },
    dark: { state: true },
    note: { state: true },
    errors: { state: true },
    logdataText: { state: true },
    menuOpen: { state: true },
  };
  declare logdataDomain: string;
  declare logdataUrl: string;
  declare hubUrl: string;
  declare panels: string;
  declare layout: string;
  declare theme: string;
  declare storageKey: string;
  declare workerBase: string;
  declare emitLogs: boolean;
  declare active: Panel;
  declare conn: ConnectionState;
  declare dark: boolean;
  declare note: string;
  declare errors: number;
  declare logdataText: string;
  declare menuOpen: boolean;

  client!: Client;
  settings!: SettingsStore;
  private notes: string[] = [];
  private noteTimer: ReturnType<typeof setTimeout> | undefined;
  private off: (() => void)[] = [];
  private media = matchMedia('(prefers-color-scheme: dark)');
  private token: string | undefined;
  private tokenAsked = false;

  constructor() {
    super();
    this.logdataDomain = '';
    this.logdataUrl = '';
    this.hubUrl = '';
    this.panels = PANELS.join(',');
    this.layout = '';
    this.theme = 'auto';
    this.storageKey = 'uclog';
    this.workerBase = '';
    this.emitLogs = false;
    this.active = 'log';
    this.conn = { link: 'idle', liveness: null, stdinWanted: false, rx: 0, tx: 0 };
    this.dark = false;
    this.note = '';
    this.errors = 0;
    this.logdataText = '';
    this.menuOpen = false;
  }

  /** The logdata service token (a property only, never an attribute). */
  get logdataToken(): string | undefined {
    return this.token;
  }
  set logdataToken(t: string | undefined) {
    this.token = t;
    this.configure();
  }

  static styles = [
    base,
    css`
      :host {
        ${LIGHT}
        display: flex;
        flex-direction: column;
        height: 100%;
        min-height: 360px;
        background: var(--uc-bg);
        border: 1px solid var(--uc-border);
        border-radius: var(--uc-radius, 8px);
        overflow: hidden;
        position: relative;
        box-sizing: border-box;
      }
      :host([data-dark]) {
        ${DARK}
      }
      header {
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 6px 8px;
        background: var(--uc-panel);
        border-bottom: 1px solid var(--uc-border);
        flex-wrap: wrap;
      }
      .brand {
        font-weight: 700;
        letter-spacing: 0.02em;
        margin-right: 4px;
      }
      nav {
        display: flex;
        gap: 2px;
        flex-wrap: wrap;
      }
      nav button {
        background: transparent;
        border-color: transparent;
      }
      nav button[aria-selected='true'] {
        background: var(--uc-button-hover);
        border-color: var(--uc-border);
        font-weight: 600;
      }
      .spacer {
        flex: 1;
      }
      main {
        flex: 1;
        min-height: 0;
        display: flex;
        background: var(--uc-panel);
      }
      main.tabs > section {
        flex: 1;
        min-width: 0;
        display: none;
      }
      main.tabs > section.on {
        display: flex;
        flex-direction: column;
      }
      main.grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(420px, 1fr));
        grid-auto-rows: minmax(300px, 1fr);
        gap: 1px;
        background: var(--uc-border);
        overflow: auto;
      }
      main.grid > section {
        background: var(--uc-panel);
        display: flex;
        flex-direction: column;
        min-height: 0;
      }
      main.grid > section > h4 {
        margin: 0;
        padding: 4px 8px;
        font-size: 0.85em;
        color: var(--uc-muted);
        border-bottom: 1px solid var(--uc-border);
      }
      section > * {
        flex: 1;
        min-height: 0;
      }
      section > h4 {
        flex: none;
      }
      footer {
        display: flex;
        gap: 14px;
        align-items: center;
        padding: 3px 10px;
        font-size: 12px;
        background: var(--uc-panel);
        border-top: 1px solid var(--uc-border);
        white-space: nowrap;
        overflow: hidden;
      }
      footer .note {
        overflow: hidden;
        text-overflow: ellipsis;
        min-width: 0;
      }
      .dot {
        display: inline-block;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        margin-right: 6px;
        background: var(--uc-muted);
        vertical-align: middle;
      }
      .dot.alive {
        background: var(--uc-ok);
      }
      .dot.warn {
        background: var(--uc-warn);
      }
      .dot.bad {
        background: var(--uc-error);
      }
      .errors {
        color: var(--uc-error);
        cursor: pointer;
        border: none;
        background: none;
        padding: 0;
        min-height: 0;
      }
      .menu {
        position: absolute;
        right: 8px;
        top: 40px;
        z-index: 5;
        background: var(--uc-panel);
        border: 1px solid var(--uc-border);
        border-radius: 8px;
        box-shadow: 0 6px 24px rgb(0 0 0 / 0.25);
        display: flex;
        flex-direction: column;
        padding: 4px;
      }
      .menu button {
        border: none;
        background: none;
        text-align: left;
      }
      .menu button:hover {
        background: var(--uc-button-hover);
      }
    `,
  ];

  connectedCallback() {
    super.connectedCallback();
    // A page that has <uc-log> before the bundle's script upgrades it while the bundle is still
    // evaluating (before it sets UcLog.workers): start a microtask later.
    if (!this.client) queueMicrotask(() => this.start());
    this.media.addEventListener('change', this.onMedia);
    this.onMedia();
  }

  private start() {
    if (this.client || !this.isConnected) return;
    const host = UcLog.host;
    this.settings = new SettingsStore(this.storageKey, host?.settings ?? null);
    const w = this.workerBase ? workersAt(this.workerBase) : UcLog.workers;
    if (!w) throw new Error('<uc-log>: set UcLog.workers or the worker-base attribute');
    this.client = new Client(w, host?.services ?? null);
    if (host?.services?.fs && host.files) this.client.setHostFiles(host.files);
    if (host?.save) setSaveHook(host.save);
    this.wire();
    this.configure();
    this.requestUpdate();
    this.dispatchEvent(new CustomEvent('uc-ready', { bubbles: true, composed: true }));
  }

  /** Resolves once the client is running (methods need it). */
  ready(): Promise<void> {
    if (this.client) return Promise.resolve();
    return new Promise((r) => this.addEventListener('uc-ready', () => r(), { once: true }));
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this.media.removeEventListener('change', this.onMedia);
    // Moving the element in the DOM disconnects and reconnects it; only dispose when it is gone.
    queueMicrotask(() => {
      if (!this.isConnected && this.client) {
        this.off.forEach((f) => f());
        this.off = [];
        this.client.dispose();
        this.client = undefined as unknown as Client;
      }
    });
  }

  private onMedia = () => {
    this.dark = this.theme === 'dark' || (this.theme !== 'light' && this.media.matches);
    this.toggleAttribute('data-dark', this.dark);
  };

  updated(ch: Map<string, unknown>) {
    if (ch.has('theme')) this.onMedia();
    if (ch.has('logdataDomain') || ch.has('logdataUrl') || ch.has('emitLogs')) this.configure();
  }

  private configure() {
    if (!this.client || !this.settings) return;
    const s = this.settings.s;
    this.client.configure(
      {
        domain: this.logdataDomain || s.logdata.domain || undefined,
        url: this.logdataUrl || s.logdata.url || undefined,
        token: this.token ?? (this.settings.token || undefined),
      },
      s.maxLines,
      this.emitLogs,
    );
  }

  /** `addEventListener`, removed with the client (an element removed and re-added starts anew). */
  private listen(t: EventTarget, type: string, f: (e: Event) => void) {
    t.addEventListener(type, f);
    this.off.push(() => t.removeEventListener(type, f));
  }

  private wire() {
    const c = this.client;
    this.listen(this.settings, 'change', () => {
      this.configure();
      this.requestUpdate();
    });
    let wasOpen = false;
    this.off.push(
      c.on('connection', (s) => {
        this.conn = s;
        const open = s.link === 'open';
        if (open && !wasOpen) {
          this.fire('uc-connect', { uri: s.uri, description: s.description });
          // Lines to send after connecting (settings).
          const lines = this.settings.s.terminal.onConnect;
          if (lines.length) setTimeout(() => lines.forEach((l) => c.send(61, `${l}\r`)), 300);
        }
        if (!open && wasOpen) this.fire('uc-disconnect', { reason: s.detail });
        if (s.link === 'busy' || s.link === 'error' || s.link === 'timeout') this.fire('uc-error', { kind: s.link, message: s.detail });
        wasOpen = open;
      }),
      c.on('device-info', (d) => this.fire('uc-device-info', d)),
      c.on('logdata', (l) => {
        this.logdataText = l.source ? `${l.source.kind} ${shorten(l.source.name)}` : 'none found';
        if (l.tokenRefused) this.tokenRefused();
      }),
      c.on('state', (s) => {
        this.errors = Object.values(s.errorCounts).reduce((a, b) => a + b, 0);
      }),
      c.on('note', (n) => this.addNote(n)),
      c.on('logs', (rows) => this.fire('uc-log', { records: rows })),
    );
    this.listen(this, 'uc-note', (e) => this.addNote((e as CustomEvent<string>).detail));
    this.listen(document, 'pointerdown', (e) => {
      if (this.menuOpen && !e.composedPath().includes(this.renderRoot.querySelector('.menu') as EventTarget)) this.menuOpen = false;
    });
  }

  /** The service refused the token but read without it worked: offer once to remove the saved
   * token. A token set by the page (`token` property) is the page's to change: only the event. */
  private tokenRefused() {
    this.fire('uc-token-refused', {});
    if (this.tokenAsked) return;
    this.tokenAsked = true;
    if (UcLog.host?.tokenRefused) return UcLog.host.tokenRefused();
    if (this.token !== undefined || !this.settings.token) return;
    const q = 'The logdata service refused the saved token, but this account allows reading without one. Remove the saved token?';
    if (confirm(q)) this.settings.setToken('', false);
  }

  private addNote(n: string) {
    this.notes = [...this.notes.slice(-99), n];
    this.note = n;
    clearTimeout(this.noteTimer);
    this.noteTimer = setTimeout(() => (this.note = ''), 8000);
    this.requestUpdate();
  }

  private fire(type: string, detail: unknown) {
    this.dispatchEvent(new CustomEvent(type, { detail, bubbles: true, composed: true }));
  }

  // -------------------------------------------------------------------------------------------
  // Public API
  // -------------------------------------------------------------------------------------------

  /** Connect: `ws://host:9000`, `loopback:`, `serial`/`hid`/`ble` (chooser), or a device object. */
  async connect(spec: LinkSpec) {
    await this.ready();
    return this.client.connect(spec);
  }

  async disconnect() {
    await this.ready();
    return this.client.disconnect();
  }

  send(port: number, data: Uint8Array | string) {
    this.client?.send(port, data);
  }

  async loadLogdata(file: File | { name: string; data: Uint8Array }) {
    await this.ready();
    const data = file instanceof File ? await readFile(file) : file.data;
    return this.client.loadLogdata(file.name, data);
  }

  /** Choose the image and start the update (connected) or wait for Start (not connected). */
  async fwUpdate(name: string, data: Uint8Array) {
    await this.ready();
    if (!this.enabled.includes('fw-update')) throw new Error('the fw-update panel is not enabled');
    this.active = 'fw-update';
    await this.updateComplete;
    await (this.renderRoot.querySelector('uc-fw-update') as UcFwUpdate).startWith(name, data);
  }

  /** Show the connect dialog (or the host's). */
  showPicker() {
    if (UcLog.host?.showConnect) return UcLog.host.showConnect();
    const p = this.renderRoot.querySelector('uc-device-picker') as UcDevicePicker;
    if (this.hubUrl && !this.settings.s.recentHubs.length) this.settings.s.lastHub = this.hubUrl;
    void p.show();
  }

  /** Zip of the logs (CSV), the terminal text and every plot (CSV). */
  async exportAll() {
    const t = stamp();
    const term = this.renderRoot.querySelector('uc-terminal') as UcTerminal | null;
    const plots = this.renderRoot.querySelector('uc-plots') as UcPlots | null;
    const files = [
      { name: `uclog-${t}-logs.csv`, data: await this.client.exportLogs() },
      { name: `uclog-${t}-terminal.txt`, data: term?.text() ?? '' },
      ...((await plots?.exportAll()) ?? []).map((p) => ({ name: `uclog-${t}-${p.name}`, data: p.data })),
    ];
    download(`uclog-${t}.zip`, new Blob([zip(files) as BlobPart], { type: 'application/zip' }));
  }

  // -------------------------------------------------------------------------------------------

  private get enabled(): Panel[] {
    const want = this.panels
      .split(',')
      .map((p) => p.trim())
      .filter(Boolean);
    const list = PANELS.filter((p) => want.includes(p));
    return list.length ? list : [...PANELS];
  }

  private status() {
    const c = this.conn;
    let dot = '';
    let text = 'not connected';
    switch (c.link) {
      case 'connecting':
        dot = 'warn';
        text = `connecting ${c.uri ?? ''}…`;
        break;
      case 'approval':
        dot = 'warn';
        text = 'waiting for hub approval…';
        break;
      case 'open':
        dot = c.liveness === 'alive' ? 'alive' : c.liveness === 'not-responding' ? 'bad' : 'warn';
        text = `${c.description ?? c.uri ?? ''}${c.liveness === 'not-responding' ? ' — device not responding' : c.liveness === 'waiting' ? ' — waiting for data' : ''}`;
        break;
      case 'busy':
        dot = 'bad';
        text = 'hub busy: another client is connected';
        break;
      case 'error':
      case 'timeout':
        dot = 'bad';
        text = c.detail ?? 'connection failed';
        break;
      case 'closed':
        text = c.detail && c.detail !== 'disconnected' ? `disconnected (${c.detail})` : 'disconnected';
        break;
    }
    return { dot, text };
  }

  private panel(p: Panel) {
    switch (p) {
      case 'log':
        return html`<uc-log-view .client=${this.client} .settings=${this.settings} .host=${UcLog.host} ?dark=${this.dark}></uc-log-view>`;
      case 'terminal':
        return html`<uc-terminal .client=${this.client} .settings=${this.settings} ?dark=${this.dark}></uc-terminal>`;
      case 'commands':
        return html`<uc-commands .client=${this.client} .settings=${this.settings}></uc-commands>`;
      case 'plots':
        return html`<uc-plots .client=${this.client} ?dark=${this.dark}></uc-plots>`;
      case 'device':
        return html`<uc-device-info .client=${this.client} .notes=${this.notes} .host=${UcLog.host}></uc-device-info>`;
      case 'fw-update':
        return html`<uc-fw-update .client=${this.client} .host=${UcLog.host}></uc-fw-update>`;
    }
  }

  render() {
    if (!this.client) return nothing;
    const panels = this.enabled;
    if (!panels.includes(this.active)) this.active = panels[0];
    const layout = (this.layout || this.settings.s.layout) === 'grid' ? 'grid' : 'tabs';
    const st = this.status();
    const connected = this.conn.link === 'open' || this.conn.link === 'connecting' || this.conn.link === 'approval';
    return html`
      <header part="header">
        <span class="brand">ucLog</span>
        ${connected
          ? html`<button @click=${() => this.disconnect()}>Disconnect</button>`
          : html`<button class="primary" @click=${() => this.showPicker()}>Connect…</button>`}
        ${layout === 'tabs'
          ? html`<nav role="tablist" part="tabs">
              ${panels.map(
                (p) => html`<button role="tab" aria-selected=${p === this.active} @click=${() => (this.active = p)}>${TITLES[p]}</button>`,
              )}
            </nav>`
          : nothing}
        <span class="spacer"></span>
        <button aria-haspopup="menu" aria-expanded=${this.menuOpen} @click=${() => (this.menuOpen = !this.menuOpen)}>⋯</button>
      </header>
      ${this.menuOpen
        ? html`<div class="menu" role="menu">
            <button
              @click=${() => {
                this.menuOpen = false;
                void this.exportAll();
              }}
            >
              Export all (zip)
            </button>
            <button
              @click=${() => {
                this.menuOpen = false;
                this.settings.s.layout = layout === 'grid' ? 'tabs' : 'grid';
                this.layout = '';
                this.settings.save();
              }}
            >
              ${layout === 'grid' ? 'Tabs layout' : 'Grid layout'}
            </button>
            <button
              @click=${() => {
                this.menuOpen = false;
                if (UcLog.host?.openSettings) UcLog.host.openSettings();
                else (this.renderRoot.querySelector('uc-settings') as UcSettings).show();
              }}
            >
              Settings…
            </button>
          </div>`
        : nothing}
      <main class=${layout} part="main">
        ${panels.map(
          (p) => html`<section class=${p === this.active ? 'on' : ''} part="panel panel-${p}" aria-label=${TITLES[p]}>
            ${layout === 'grid' ? html`<h4>${TITLES[p]}</h4>` : nothing} ${this.panel(p)}
          </section>`,
        )}
      </main>
      <footer part="status">
        <span><span class="dot ${st.dot}"></span>${st.text}</span>
        ${this.conn.stdinWanted ? html`<span class="err">device waits for stdin</span>` : nothing}
        ${this.logdataText ? html`<span>logdata: ${this.logdataText}</span>` : nothing}
        ${this.errors
          ? html`<button class="errors" title="Error records (see the Device view)" @click=${() => (this.active = 'device')}>
              ${this.errors} error${this.errors === 1 ? '' : 's'}
            </button>`
          : nothing}
        <span class="note muted">${this.note}</span>
      </footer>
      <uc-device-picker .client=${this.client} .settings=${this.settings}></uc-device-picker>
      <uc-settings .settings=${this.settings}></uc-settings>
    `;
  }
}

function shorten(s: string): string {
  s = s.replace(/([0-9a-f]{8})[0-9a-f]{120}/i, '$1…');
  if (s.length <= 48) return s;
  const parts = s.split('/');
  return parts.length > 2 ? `…/${parts.slice(-2).join('/')}` : `${s.slice(0, 22)}…${s.slice(-22)}`;
}

customElements.define('uc-log', UcLog);

declare global {
  interface HTMLElementTagNameMap {
    'uc-log': UcLog;
  }
}
