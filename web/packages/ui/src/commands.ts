// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * `<uc-commands>`: port 0 (plan 02 §6). Configurable buttons (a text line, or a CBOR map whose
 * `$name` placeholders get an input next to the button), a free text line, and the replies.
 */

import { css, html, LitElement, nothing } from 'lit';
import { display } from '@uclog/core';
import type { Client } from '@uclog/workers';
import { base } from './styles.ts';
import { buttonArgs, buttonPayload, type SettingsStore } from './settings.ts';

export class UcCommands extends LitElement {
  static properties = {
    client: { attribute: false },
    settings: { attribute: false },
    responses: { state: true },
    free: { state: true },
    answers: { state: true },
  };
  declare client: Client;
  declare settings: SettingsStore;
  declare responses: { at: number; text: string; sent?: boolean }[];
  declare free: string;
  declare answers: Record<string, string>;
  private off: (() => void)[] = [];
  private partial = '';

  constructor() {
    super();
    this.responses = [];
    this.free = '';
    this.answers = {};
  }

  static styles = [
    base,
    css`
      :host {
        display: flex;
        flex-direction: column;
        height: 100%;
        min-height: 0;
        padding: 8px;
        box-sizing: border-box;
        gap: 8px;
      }
      .buttons {
        display: flex;
        flex-direction: column;
        gap: 6px;
      }
      .arg input {
        width: 7em;
      }
      .free input {
        flex: 1;
      }
      .out {
        flex: 1;
        overflow: auto;
        border: 1px solid var(--uc-border);
        border-radius: 6px;
        padding: 6px 8px;
        white-space: pre-wrap;
        background: var(--uc-term-bg);
      }
      .out .sent {
        color: var(--uc-muted);
      }
    `,
  ];

  private onSettings = () => this.requestUpdate();

  connectedCallback() {
    super.connectedCallback();
    this.settings.addEventListener('change', this.onSettings);
    this.off.push(() => this.settings.removeEventListener('change', this.onSettings));
    this.off.push(
      this.client.on('command', (c) => {
        if (c.value !== undefined) return this.push(display(c.value));
        // Text replies may arrive in pieces; show whole lines.
        this.partial += c.text ?? '';
        const lines = this.partial.split(/\r?\n/);
        this.partial = lines.pop() ?? '';
        for (const l of lines) if (l.trim()) this.push(l);
      }),
    );
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this.off.forEach((f) => f());
    this.off = [];
  }

  private push(text: string, sent = false) {
    this.responses = [...this.responses.slice(-499), { at: Date.now(), text, sent }];
    void this.updateComplete.then(() => {
      const o = this.renderRoot.querySelector('.out');
      if (o) o.scrollTop = o.scrollHeight;
    });
  }

  /** A text line, or a JSON object sent as CBOR (like the native GUI). */
  private sendFree() {
    const l = this.free.trim();
    if (!l) return;
    if (l.startsWith('{')) {
      let v: unknown;
      try {
        v = JSON.parse(l);
      } catch (e) {
        this.push(`invalid JSON: ${(e as Error).message}`);
        return;
      }
      this.client.send(0, buttonPayload({ label: '', cmd: v as Record<string, unknown> }));
    } else this.client.command(l);
    this.push(`› ${l}`, true);
    this.free = '';
  }

  render() {
    const buttons = this.settings.s.commands.buttons;
    return html`
      <div class="buttons">
        ${buttons.length
          ? buttons.map((b) => {
              const args = buttonArgs(b);
              return html`<div class="row">
                <button
                  @click=${() => {
                    this.client.send(0, buttonPayload(b, this.answers));
                    this.push(`› ${b.label}`, true);
                  }}
                >
                  ${b.label}
                </button>
                ${args.map(
                  ([k, a]) => html`<label class="arg"
                    >${k} ${a}
                    <input
                      .value=${this.answers[`${k} ${a}`] ?? ''}
                      @input=${(e: Event) => (this.answers = { ...this.answers, [`${k} ${a}`]: (e.target as HTMLInputElement).value })}
                  /></label>`,
                )}
              </div>`;
            })
          : html`<span class="muted">No command buttons. Add them in Settings.</span>`}
      </div>
      <div class="row free">
        <input
          class="mono"
          placeholder="text line, or a JSON object sent as CBOR"
          aria-label="Command"
          .value=${this.free}
          @input=${(e: Event) => (this.free = (e.target as HTMLInputElement).value)}
          @keydown=${(e: KeyboardEvent) => e.key === 'Enter' && this.sendFree()}
        />
        <button @click=${() => this.sendFree()}>Send</button>
        <button @click=${() => (this.responses = [])}>Clear</button>
      </div>
      <div class="out mono" part="responses" aria-live="polite">
        ${this.responses.length
          ? this.responses.map((r) => html`<div class=${r.sent ? 'sent' : ''}>${r.text}</div>`)
          : html`<span class="muted">Replies on port 0 appear here.</span>`}
      </div>
      ${nothing}
    `;
  }
}

customElements.define('uc-commands', UcCommands);
