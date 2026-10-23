// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * `<uc-device-info>`: port-62 fields, the logdata in use and how it was found, manual logdata
 * loading, the semihost files folder, and counters (plan 02 §6).
 */

import { css, html, LitElement, nothing } from 'lit';
import type { Client } from '@uclog/workers';
import { base } from './styles.ts';
import { readFile } from './util.ts';

export class UcDeviceInfo extends LitElement {
  static properties = {
    client: { attribute: false },
    notes: { attribute: false },
    loadError: { state: true },
    tick: { state: true },
  };
  declare client: Client;
  declare notes: string[];
  declare loadError: string;
  declare tick: number;
  private off: (() => void)[] = [];

  constructor() {
    super();
    this.notes = [];
    this.loadError = '';
    this.tick = 0;
  }

  static styles = [
    base,
    css`
      :host {
        display: block;
        height: 100%;
        overflow: auto;
        padding: 8px 12px;
        box-sizing: border-box;
      }
      table {
        border-collapse: collapse;
      }
      td {
        padding: 3px 16px 3px 0;
        vertical-align: top;
      }
      td:first-child {
        color: var(--uc-muted);
        white-space: nowrap;
      }
      td.v {
        word-break: break-all;
      }
      ul {
        margin: 4px 0;
        padding-left: 18px;
      }
    `,
  ];

  connectedCallback() {
    super.connectedCallback();
    const bump = () => this.tick++;
    this.off.push(
      this.client.on('device-info', bump),
      this.client.on('logdata', bump),
      this.client.on('connection', bump),
      this.client.on('state', bump),
    );
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this.off.forEach((f) => f());
    this.off = [];
  }

  private async load(e: Event) {
    const input = e.target as HTMLInputElement;
    const f = input.files?.[0];
    input.value = '';
    if (!f) return;
    try {
      const r = await this.client.loadLogdata(f.name, await readFile(f));
      this.loadError = '';
      this.dispatchEvent(new CustomEvent('uc-note', { detail: `logdata ${f.name} loaded for target ${r.target}`, bubbles: true, composed: true }));
    } catch (err) {
      this.loadError = (err as Error).message;
    }
  }

  private async pickFolder() {
    try {
      const h = await (window as unknown as { showDirectoryPicker(o: object): Promise<FileSystemDirectoryHandle> }).showDirectoryPicker({
        mode: 'readwrite',
        id: 'uclog-files',
      });
      this.client.setFilesDir(h);
      this.tick++;
    } catch {
      /* cancelled */
    }
  }

  render() {
    const c = this.client.conn;
    const info = this.client.deviceInfo;
    const ld = this.client.logdata;
    const st = this.client.decodeState;
    const fs = this.client.filesDir;
    const canPickDir = 'showDirectoryPicker' in window;
    return html`
      <table>
        <tr>
          <td>Connection</td>
          <td>${c.description ?? c.uri ?? '—'} ${c.uri && c.uri.startsWith('ws') ? html`<span class="muted">(hub)</span>` : nothing}</td>
        </tr>
        ${info
          ? info.fields.map(([k, v]) => html`<tr><td>${k}</td><td class="v mono">${v}</td></tr>`)
          : html`<tr>
              <td>Device info</td>
              <td class="muted">none yet (it arrives on port 62)</td>
            </tr>`}
        <tr>
          <td>Logdata</td>
          <td>
            ${ld?.source
              ? html`${ld.source.kind}: <span class="mono">${ld.source.name.replace(/([0-9a-f]{8})[0-9a-f]{120}/i, '$1…')}</span> (target${ld.targets.length > 1 ? 's' : ''}
                  ${ld.targets.join(', ')})`
              : ld
                ? html`<span class="err">none found for this app_hash</span>`
                : html`<span class="muted">—</span>`}
          </td>
        </tr>
        <tr>
          <td>Semihost files</td>
          <td>
            ${fs ? html`serving <b>${fs.name}</b>` : html`<span class="muted">no folder: file ops fail with EACCES</span>`}
          </td>
        </tr>
        <tr>
          <td>Traffic</td>
          <td>${c.rx.toLocaleString()} bytes received, ${c.tx.toLocaleString()} sent</td>
        </tr>
        ${st
          ? html`<tr>
              <td>Logs</td>
              <td>
                ${st.received.toLocaleString()} received, ${st.stored.toLocaleString()} stored, ${st.visible.toLocaleString()} shown${st.discarded
                  ? `, ${st.discarded.toLocaleString()} discarded`
                  : ''}
              </td>
            </tr>`
          : nothing}
        ${st && Object.keys(st.errorCounts).length
          ? html`<tr>
              <td>Error records</td>
              <td>${Object.entries(st.errorCounts).map(([k, n]) => html`<div><span class="mono">${k}</span>: ${n}</div>`)}</td>
            </tr>`
          : nothing}
      </table>
      <div class="row" style="margin-top:10px">
        <label class="row">
          <button @click=${(e: Event) => ((e.currentTarget as HTMLElement).nextElementSibling as HTMLInputElement).click()}>Load logdata…</button>
          <input type="file" accept=".logdata,.elf,application/octet-stream" hidden @change=${this.load} />
        </label>
        ${canPickDir
          ? html`<button @click=${this.pickFolder} title="Let the device open and create files in a folder (semihost)">Serve files from folder…</button>`
          : nothing}
        ${fs ? html`<button @click=${() => (this.client.setFilesDir(null), this.tick++)}>Stop serving files</button>` : nothing}
      </div>
      ${this.loadError ? html`<p class="err">${this.loadError}</p>` : nothing}
      ${this.notes.length
        ? html`<h3>Notes</h3>
            <ul class="muted">
              ${this.notes
                .slice(-30)
                .reverse()
                .map((n) => html`<li>${n}</li>`)}
            </ul>`
        : nothing}
    `;
  }
}

customElements.define('uc-device-info', UcDeviceInfo);
