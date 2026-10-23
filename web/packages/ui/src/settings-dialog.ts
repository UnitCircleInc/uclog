// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** `<uc-settings>`: logdata service, max log lines, on-connect lines, the button lists (JSON). */

import { css, html, LitElement, nothing } from 'lit';
import { DEFAULT_LOGDATA_URL } from '@uclog/core';
import { base } from './styles.ts';
import type { Button, SettingsStore } from './settings.ts';

export class UcSettings extends LitElement {
  static properties = {
    settings: { attribute: false },
    error: { state: true },
  };
  declare settings: SettingsStore;
  declare error: string;

  constructor() {
    super();
    this.error = '';
  }

  static styles = [
    base,
    css`
      dialog {
        border: 1px solid var(--uc-border);
        border-radius: 10px;
        background: var(--uc-panel);
        color: var(--uc-fg);
        padding: 16px 18px;
        width: min(640px, calc(100vw - 32px));
        box-shadow: 0 10px 40px rgb(0 0 0 / 0.3);
      }
      dialog::backdrop {
        background: rgb(0 0 0 / 0.35);
      }
      h2 {
        font-size: 1.15em;
        margin: 0 0 10px;
      }
      .grid {
        display: grid;
        grid-template-columns: max-content 1fr;
        gap: 8px 12px;
        align-items: center;
      }
      textarea {
        width: 100%;
        min-height: 5em;
        font-family: var(--uc-mono, ui-monospace, Menlo, Consolas, monospace);
        font-size: 12px;
      }
      .foot {
        display: flex;
        gap: 8px;
        justify-content: flex-end;
        margin-top: 14px;
      }
    `,
  ];

  show() {
    this.error = '';
    this.requestUpdate();
    void this.updateComplete.then(() => this.renderRoot.querySelector('dialog')?.showModal());
  }

  private close() {
    this.renderRoot.querySelector('dialog')?.close();
  }

  private val(id: string) {
    return (this.renderRoot.querySelector(`#${id}`) as HTMLInputElement | HTMLTextAreaElement).value;
  }

  private save() {
    const parse = (id: string, what: string): Button[] => {
      let v: unknown;
      try {
        v = JSON.parse(this.val(id) || '[]');
      } catch (e) {
        throw new Error(`${what}: ${(e as Error).message}`);
      }
      if (!Array.isArray(v) || !v.every((b) => b && typeof b.label === 'string' && (typeof b.cmd === 'string' || (typeof b.cmd === 'object' && b.cmd)))) {
        throw new Error(`${what}: expected [{"label": "…", "cmd": "text or {…}"}]`);
      }
      return v as Button[];
    };
    try {
      const s = this.settings.s;
      const tb = parse('tb', 'Terminal buttons');
      const cb = parse('cb', 'Command buttons');
      s.logdata.domain = this.val('domain').trim();
      s.logdata.url = this.val('url').trim();
      s.maxLines = Math.min(10_000_000, Math.max(100, Number(this.val('max')) || 100_000));
      s.terminal.onConnect = this.val('onc')
        .split('\n')
        .map((l) => l.trim())
        .filter(Boolean);
      s.terminal.buttons = tb;
      s.commands.buttons = cb;
      const remember = (this.renderRoot.querySelector('#remember') as HTMLInputElement).checked;
      this.settings.setToken(this.val('token').trim(), remember);
      this.settings.save();
      this.close();
    } catch (e) {
      this.error = (e as Error).message;
    }
  }

  render() {
    const s = this.settings.s;
    return html`<dialog aria-label="Settings">
      <h2>Settings</h2>
      <div class="grid">
        <label for="domain">Logdata domain</label>
        <input id="domain" .value=${s.logdata.domain} placeholder="e.g. acme.example (needed for lookups)" />
        <label for="url">Logdata URL</label>
        <input id="url" .value=${s.logdata.url} placeholder=${DEFAULT_LOGDATA_URL} />
        <label for="token">Logdata token</label>
        <div class="row">
          <input id="token" type="password" autocomplete="off" .value=${this.settings.token} style="flex:1" />
          <label><input id="remember" type="checkbox" .checked=${this.settings.rememberToken} /> Remember</label>
        </div>
        <label for="max">Max log lines</label>
        <input id="max" type="number" min="100" step="1000" .value=${String(s.maxLines)} />
        <label for="onc">Terminal on connect</label>
        <textarea id="onc" placeholder="one line each, e.g. version" .value=${s.terminal.onConnect.join('\n')}></textarea>
      </div>
      <h3>Terminal buttons</h3>
      <textarea id="tb" .value=${JSON.stringify(s.terminal.buttons, null, 1)}></textarea>
      <h3>Command buttons <span class="muted">("cmd": a text line, or an object sent as CBOR with $args)</span></h3>
      <textarea id="cb" .value=${JSON.stringify(s.commands.buttons, null, 1)}></textarea>
      ${this.error ? html`<p class="err">${this.error}</p>` : nothing}
      <div class="foot">
        <button @click=${() => this.close()}>Cancel</button>
        <button class="primary" @click=${() => this.save()}>Save</button>
      </div>
    </dialog>`;
  }
}

customElements.define('uc-settings', UcSettings);
