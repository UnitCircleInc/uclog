// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * `<uc-device-picker>`: one list (plan 02 §5) with the USB devices this site may already use, the
 * browser's choosers to add a Serial/HID/Bluetooth device, hubs by URL (recent ones remembered;
 * browsers cannot browse mDNS), and the simulated demo device.
 */

import { css, html, LitElement, nothing } from 'lit';
import { grantedDevices, serialDescription, support } from '@uclog/transports-web';
import type { Client, LinkSpec } from '@uclog/workers';
import { base } from './styles.ts';
import type { SettingsStore } from './settings.ts';

export class UcDevicePicker extends LitElement {
  static properties = {
    client: { attribute: false },
    settings: { attribute: false },
    open: { type: Boolean, reflect: true },
    serial: { state: true },
    hid: { state: true },
    hub: { state: true },
    error: { state: true },
    busy: { state: true },
  };
  declare client: Client;
  declare settings: SettingsStore;
  declare open: boolean;
  declare serial: SerialPort[];
  declare hid: HIDDevice[];
  declare hub: string;
  declare error: string;
  declare busy: boolean;

  constructor() {
    super();
    this.open = false;
    this.serial = [];
    this.hid = [];
    this.hub = '';
    this.error = '';
    this.busy = false;
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
        width: min(560px, calc(100vw - 32px));
        box-shadow: 0 10px 40px rgb(0 0 0 / 0.3);
      }
      dialog::backdrop {
        background: rgb(0 0 0 / 0.35);
      }
      h2 {
        font-size: 1.15em;
        margin: 0 0 8px;
      }
      ul {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 4px;
      }
      li {
        display: flex;
        align-items: center;
        gap: 8px;
        justify-content: space-between;
        padding: 4px 8px;
        border: 1px solid var(--uc-border);
        border-radius: 6px;
      }
      .kind {
        font-size: 0.85em;
        color: var(--uc-muted);
        width: 3.5em;
        flex: none;
      }
      .grow {
        flex: 1;
      }
      input.url {
        flex: 1;
        min-width: 14em;
      }
      .foot {
        display: flex;
        justify-content: flex-end;
        margin-top: 14px;
      }
    `,
  ];

  async show() {
    this.open = true;
    this.error = '';
    this.hub ||= this.settings.s.lastHub || 'ws://localhost:9000';
    await this.updateComplete;
    this.renderRoot.querySelector('dialog')?.showModal();
    await this.refresh();
  }

  close() {
    this.open = false;
    this.renderRoot.querySelector('dialog')?.close();
  }

  private async refresh() {
    const g = await grantedDevices().catch(() => ({ serial: [], hid: [] }));
    this.serial = g.serial;
    this.hid = g.hid;
  }

  private async go(spec: LinkSpec) {
    this.busy = true;
    this.error = '';
    try {
      await this.client.connect(spec);
      if (typeof spec === 'string' && /^wss?:/.test(spec)) this.settings.addHub(spec);
      this.close();
      this.dispatchEvent(new CustomEvent('uc-picked', { bubbles: true, composed: true }));
    } catch (e) {
      const err = e as DOMException;
      // The user closing the chooser is not an error.
      if (err.name !== 'NotFoundError' && err.name !== 'AbortError') this.error = err.message || String(e);
    } finally {
      this.busy = false;
    }
  }

  render() {
    const s = support();
    const hubs = this.settings.s.recentHubs;
    const secure = window.isSecureContext;
    return html`<dialog @close=${() => (this.open = false)} aria-label="Connect">
      <h2>Connect</h2>
      ${secure ? nothing : html`<p class="err">This page is not a secure context: Web Serial, WebHID and Web Bluetooth need https.</p>`}
      <h3>USB devices you allowed</h3>
      ${this.serial.length || this.hid.length
        ? html`<ul>
            ${this.serial.map(
              (p) => html`<li>
                <span class="kind">Serial</span><span class="grow">${serialDescription(p)}</span>
                <button ?disabled=${this.busy} @click=${() => this.go({ kind: 'serial', port: p })}>Connect</button>
              </li>`,
            )}
            ${this.hid.map(
              (d) => html`<li>
                <span class="kind">HID</span><span class="grow">${d.productName || 'HID device'}</span>
                <button ?disabled=${this.busy} @click=${() => this.go({ kind: 'hid', device: d })}>Connect</button>
              </li>`,
            )}
          </ul>`
        : html`<p class="muted">None yet. Add one below; the browser remembers it for this site.</p>`}
      <h3>Add a device</h3>
      <div class="row">
        <button ?disabled=${!s.serial || this.busy} @click=${() => this.go('serial')} title=${s.serial ? '' : 'Web Serial is not available in this browser'}>
          Serial…
        </button>
        <button ?disabled=${!s.hid || this.busy} @click=${() => this.go('hid')} title=${s.hid ? '' : 'WebHID is not available in this browser'}>HID…</button>
        <button ?disabled=${!s.ble || this.busy} @click=${() => this.go('ble')} title=${s.ble ? '' : 'Web Bluetooth is not available in this browser'}>
          Bluetooth…
        </button>
        ${!s.serial && !s.hid ? html`<span class="muted">Use Chrome or Edge for USB devices.</span>` : nothing}
      </div>
      <h3>Hub</h3>
      <div class="row">
        <input
          class="url mono"
          aria-label="Hub URL"
          placeholder="ws://localhost:9000"
          .value=${this.hub}
          @input=${(e: Event) => (this.hub = (e.target as HTMLInputElement).value)}
          @keydown=${(e: KeyboardEvent) => e.key === 'Enter' && this.hub.trim() && this.go(this.hub.trim())}
        />
        <button ?disabled=${this.busy || !this.hub.trim()} @click=${() => this.go(this.hub.trim())}>Connect to hub</button>
      </div>
      ${hubs.length
        ? html`<ul style="margin-top:6px">
            ${hubs.map(
              (u) => html`<li>
                <span class="kind">Hub</span><span class="grow mono">${u}</span>
                <button ?disabled=${this.busy} @click=${() => this.go(u)}>Connect</button>
              </li>`,
            )}
          </ul>`
        : nothing}
      <h3>Demo</h3>
      <div class="row">
        <button ?disabled=${this.busy} @click=${() => this.go('loopback:rate=5')}>Simulated device</button>
        <span class="muted">Logs with spec/vectors/sample.logdata; type <span class="mono">help</span> in the terminal.</span>
      </div>
      ${this.error ? html`<p class="err">${this.error}</p>` : nothing}
      <div class="foot"><button @click=${() => this.close()}>Close</button></div>
    </dialog>`;
  }
}

customElements.define('uc-device-picker', UcDevicePicker);
