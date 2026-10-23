// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * `<uc-terminal>`: port 61 (plan 02 §6). xterm.js shows the device's VT100 output and semihost
 * stdout/stderr. Line mode (default): an input line with history; Enter sends `line\r`, or answers
 * the device's stdin read when one is pending. Character mode sends each key as typed.
 */

import { css, html, LitElement, nothing, unsafeCSS } from 'lit';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import xtermCss from '@xterm/xterm/css/xterm.css?inline';
import type { Client, ConnectionState } from '@uclog/workers';
import { base } from './styles.ts';
import { buttonPayload, type SettingsStore } from './settings.ts';
import { download, stamp, stripAnsi } from './util.ts';

export class UcTerminal extends LitElement {
  static properties = {
    client: { attribute: false },
    settings: { attribute: false },
    dark: { type: Boolean },
    charMode: { state: true },
    stdinWanted: { state: true },
    input: { state: true },
  };
  declare client: Client;
  declare settings: SettingsStore;
  declare dark: boolean;
  declare charMode: boolean;
  declare stdinWanted: boolean;
  declare input: string;

  term: Terminal | null = null;
  private fit = new FitAddon();
  private ro: ResizeObserver | null = null;
  private off: (() => void)[] = [];
  private hist: number | null = null;
  /** Everything written, for export (xterm's buffer only keeps the scrollback). */
  private log: string[] = [];
  private logLen = 0;
  private pending: Uint8Array[] = [];
  private td = new TextDecoder();

  constructor() {
    super();
    this.dark = false;
    this.charMode = false;
    this.stdinWanted = false;
    this.input = '';
  }

  static styles = [
    base,
    unsafeCSS(xtermCss),
    css`
      :host {
        display: flex;
        flex-direction: column;
        height: 100%;
        min-height: 0;
      }
      .bar {
        padding: 6px 8px;
        border-bottom: 1px solid var(--uc-border);
      }
      .term {
        flex: 1;
        min-height: 0;
        padding: 4px 0 0 6px;
        background: var(--uc-term-bg);
      }
      .input {
        padding: 6px 8px;
        border-top: 1px solid var(--uc-border);
      }
      .input input {
        flex: 1;
        min-width: 10em;
      }
      .prompt.stdin {
        color: var(--uc-warn);
        font-weight: 600;
      }
      .sep {
        width: 1px;
        align-self: stretch;
        background: var(--uc-border);
      }
    `,
  ];

  private onSettings = () => this.requestUpdate();

  connectedCallback() {
    super.connectedCallback();
    this.settings.addEventListener('change', this.onSettings);
    this.off.push(() => this.settings.removeEventListener('change', this.onSettings));
    this.off.push(
      this.client.on('terminal', (d) => this.write(d)),
      this.client.on('console', (c) => {
        // Semihost stdout/stderr use C newlines; the terminal needs CR LF. stderr is red.
        const data = crlf(c.data);
        if (c.fid === 2) this.write(new TextEncoder().encode('\x1b[31m'), data, new TextEncoder().encode('\x1b[0m'));
        else this.write(data);
      }),
      this.client.on('connection', (c: ConnectionState) => {
        if (c.stdinWanted !== this.stdinWanted) {
          this.stdinWanted = c.stdinWanted;
          if (c.stdinWanted) this.focusInput();
        }
      }),
    );
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this.off.forEach((f) => f());
    this.off = [];
    this.ro?.disconnect();
    this.term?.dispose();
    this.term = null;
  }

  firstUpdated() {
    const el = this.renderRoot.querySelector('.term') as HTMLElement;
    this.term = new Terminal({
      convertEol: false,
      scrollback: 10_000,
      fontSize: 13,
      fontFamily: 'ui-monospace, "SF Mono", Menlo, Consolas, monospace',
      cursorBlink: false,
      disableStdin: false,
      theme: this.theme(),
    });
    this.term.loadAddon(this.fit);
    this.term.open(el);
    this.term.onData((d) => {
      if (this.charMode) this.client.send(61, d);
    });
    this.ro = new ResizeObserver(() => {
      if (el.clientWidth > 0) this.fit.fit();
    });
    this.ro.observe(el);
    for (const p of this.pending) this.term.write(p);
    this.pending = [];
  }

  updated(changed: Map<string, unknown>) {
    if (changed.has('dark') && this.term) this.term.options.theme = this.theme();
  }

  private theme() {
    return this.dark
      ? { background: '#16181d', foreground: '#d7dae0', cursor: '#d7dae0', selectionBackground: '#3a4a6b' }
      : { background: '#fbfbfc', foreground: '#1d1f24', cursor: '#1d1f24', selectionBackground: '#b9d2ff' };
  }

  write(...parts: Uint8Array[]) {
    for (const p of parts) {
      const s = this.td.decode(p, { stream: true });
      this.log.push(s);
      this.logLen += s.length;
      while (this.logLen > 2_000_000 && this.log.length > 1) this.logLen -= this.log.shift()!.length;
      if (this.term) this.term.write(p);
      else this.pending.push(p);
    }
  }

  /** The terminal text so far, without escape sequences (exports). */
  text(): string {
    return stripAnsi(this.log.join('')).replace(/\r\n/g, '\n');
  }

  clear() {
    this.term?.clear();
    this.log = [];
    this.logLen = 0;
  }

  private focusInput() {
    void this.updateComplete.then(() => (this.renderRoot.querySelector('.input input') as HTMLInputElement | null)?.focus());
  }

  private submit() {
    const line = this.input;
    this.input = '';
    this.hist = null;
    if (this.stdinWanted) {
      this.write(new TextEncoder().encode(`${line}\r\n`));
      this.client.stdin(`${line}\n`);
      this.stdinWanted = false;
      return;
    }
    this.client.send(61, `${line}\r`);
    if (line.trim()) this.settings.pushHistory(line);
  }

  private key(e: KeyboardEvent) {
    const h = this.settings.s.terminal.history;
    if (e.key === 'Enter') {
      e.preventDefault();
      this.submit();
    } else if (e.key === 'ArrowUp' && h.length) {
      e.preventDefault();
      this.hist = this.hist === null ? h.length - 1 : Math.max(0, this.hist - 1);
      this.input = h[this.hist];
    } else if (e.key === 'ArrowDown' && this.hist !== null) {
      e.preventDefault();
      if (this.hist + 1 < h.length) {
        this.hist++;
        this.input = h[this.hist];
      } else {
        this.hist = null;
        this.input = '';
      }
    }
  }

  render() {
    return html`
      <div class="bar row" part="toolbar">
        ${this.settings.s.terminal.buttons.map(
          (b) => html`<button title=${typeof b.cmd === 'string' ? b.cmd : JSON.stringify(b.cmd)} @click=${() => this.client.send(61, buttonPayload(b))}>
            ${b.label}
          </button>`,
        )}
        <span class="sep"></span>
        <label title="Send each key as it is typed (for shells and editors on the device)">
          <input
            type="checkbox"
            .checked=${this.charMode}
            @change=${(e: Event) => {
              this.charMode = (e.target as HTMLInputElement).checked;
              if (this.charMode) this.term?.focus();
            }}
          />
          Character mode
        </label>
        <button @click=${() => this.clear()}>Clear</button>
        <button @click=${() => navigator.clipboard.writeText(this.text()).catch(() => {})}>Copy</button>
        <button @click=${() => download(`uclog-${stamp()}-terminal.txt`, this.text())}>Export</button>
      </div>
      <div class="term" part="terminal"></div>
      ${this.charMode && !this.stdinWanted
        ? nothing
        : html`<div class="input row">
            <span class="prompt ${this.stdinWanted ? 'stdin' : ''}">${this.stdinWanted ? 'stdin ›' : '›'}</span>
            <input
              class="mono"
              aria-label=${this.stdinWanted ? 'Line for the device stdin' : 'Terminal command'}
              placeholder=${this.stdinWanted ? "line for the device's stdin" : 'command, Enter to send'}
              .value=${this.input}
              @input=${(e: Event) => (this.input = (e.target as HTMLInputElement).value)}
              @keydown=${(e: KeyboardEvent) => this.key(e)}
            />
            <button @click=${() => this.submit()}>Send</button>
          </div>`}
    `;
  }
}

/** LF not preceded by CR → CR LF. */
export function crlf(d: Uint8Array): Uint8Array {
  let n = 0;
  for (let i = 0; i < d.length; i++) if (d[i] === 10 && (i === 0 || d[i - 1] !== 13)) n++;
  if (!n) return d;
  const out = new Uint8Array(d.length + n);
  let o = 0;
  for (let i = 0; i < d.length; i++) {
    if (d[i] === 10 && (i === 0 || d[i - 1] !== 13)) out[o++] = 13;
    out[o++] = d[i];
  }
  return out;
}

customElements.define('uc-terminal', UcTerminal);
