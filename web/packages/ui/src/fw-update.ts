// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** `<uc-fw-update>`: pick the signed `.bin`, then port 59 with fallback to 3, progress, cancel. */

import { css, html, LitElement, nothing } from 'lit';
import type { FwProgress } from '@uclog/core';
import type { Client } from '@uclog/workers';
import { base } from './styles.ts';
import { readFile } from './util.ts';
import type { UcHost } from './host.ts';

interface Image {
  name: string;
  size: number;
  data?: Uint8Array;
  file?: File;
}

export function fwLabel(p: FwProgress | null): [number, string] {
  if (!p) return [0, ''];
  switch (p.kind) {
    case 'probing':
      return [0, 'Finding the FW update port…'];
    case 'port':
      return [0, `Using port ${p.port}`];
    case 'starting':
      return [0, 'Starting (the device erases flash)…'];
    case 'block':
      return [p.done / p.total, `Block ${p.done} of ${p.total}`];
    case 'scheduling':
      return [1, 'Scheduling the update…'];
    case 'done':
      return [1, `Done (port ${p.port})`];
  }
}

export class UcFwUpdate extends LitElement {
  static properties = {
    client: { attribute: false },
    host: { attribute: false },
    file: { state: true },
    running: { state: true },
    progress: { state: true },
    result: { state: true },
    tick: { state: true },
  };
  declare client: Client;
  declare host: UcHost | null;
  declare file: Image | null;
  declare running: boolean;
  declare progress: FwProgress | null;
  declare result: { ok: boolean; text: string } | null;
  declare tick: number;
  private port: number | null = null;
  private off: (() => void)[] = [];

  constructor() {
    super();
    this.host = null;
    this.file = null;
    this.running = false;
    this.progress = null;
    this.result = null;
    this.tick = 0;
  }

  static styles = [
    base,
    css`
      :host {
        display: block;
        padding: 12px;
      }
      progress {
        width: 100%;
        max-width: 36em;
        height: 14px;
      }
      .box {
        display: flex;
        flex-direction: column;
        gap: 10px;
        max-width: 40em;
      }
    `,
  ];

  connectedCallback() {
    super.connectedCallback();
    this.off.push(
      this.client.on('fw', (p) => {
        this.progress = p;
        if (p.kind === 'port') this.port = p.port;
      }),
      this.client.on('fw-done', (r) => {
        this.running = false;
        this.result = r.error ? { ok: false, text: `Failed: ${r.error}` } : { ok: true, text: `Update scheduled (port ${r.port}); the device restarts into the new image.` };
      }),
      this.client.on('connection', () => this.tick++),
    );
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this.off.forEach((f) => f());
    this.off = [];
  }

  private async start() {
    const f = this.file;
    if (!f) return;
    this.running = true;
    this.result = null;
    this.progress = null;
    this.port = null;
    this.client.fwUpdate(f.data ?? (await readFile(f.file!)));
  }

  /** Choose an image (and start when connected): the host's Flash Firmware command. */
  async startWith(name: string, data: Uint8Array) {
    this.file = { name, size: data.length, data };
    this.result = null;
    if (this.client.conn.link === 'open' && !this.running) await this.start();
  }

  private async pick() {
    const f = await this.host?.pickFile?.('firmware');
    if (f) {
      this.file = { name: f.name, size: f.data.length, data: f.data };
      this.result = null;
    }
  }

  render() {
    const connected = this.client.conn.link === 'open';
    const [ratio, label] = fwLabel(this.progress);
    return html`<div class="box">
      ${this.host?.pickFile
        ? html`<div class="row">Signed image <button @click=${this.pick}>Choose image…</button></div>`
        : html`<label class="row"
            >Signed image
            <input
              type="file"
              accept=".bin"
              @change=${(e: Event) => {
                const f = (e.target as HTMLInputElement).files?.[0];
                this.file = f ? { name: f.name, size: f.size, file: f } : null;
                this.result = null;
              }}
            />
          </label>`}
      ${this.file ? html`<span class="muted">${this.file.name}, ${this.file.size.toLocaleString()} bytes</span>` : nothing}
      <div class="row">
        <button class="primary" ?disabled=${!connected || this.running || !this.file} @click=${this.start}>Start update</button>
        <button ?disabled=${!this.running} @click=${() => this.client.cancelFw()}>Cancel</button>
        ${connected ? nothing : html`<span class="muted">not connected</span>`}
      </div>
      ${this.progress
        ? html`<progress max="1" .value=${ratio} aria-label="FW update progress"></progress>
            <span>${label}${this.port && this.progress.kind !== 'port' && this.progress.kind !== 'done' ? ` · port ${this.port}` : ''}</span>`
        : nothing}
      ${this.result ? html`<span class=${this.result.ok ? 'ok' : 'err'}>${this.result.text}</span>` : nothing}
    </div>`;
  }
}

customElements.define('uc-fw-update', UcFwUpdate);
