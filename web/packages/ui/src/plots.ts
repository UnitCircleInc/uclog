// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * `<uc-plots>`: port 60 (plan 02 §6). One sub-tab per plot. Line plots are drawn with uPlot
 * (loaded when the first one is shown); heatmaps and spectrograms on a canvas. New plots open in
 * the background, so the plot in view stays.
 */

import { css, html, LitElement, nothing } from 'lit';
import type { Client, DecodeState, PlotInfo } from '@uclog/workers';
import { base } from './styles.ts';
import { safeName } from './util.ts';
import './grid-plot.ts';

export class UcPlots extends LitElement {
  static properties = {
    client: { attribute: false },
    dark: { type: Boolean },
    plots: { state: true },
    active: { state: true },
    lineReady: { state: true },
  };
  declare client: Client;
  declare dark: boolean;
  declare plots: PlotInfo[];
  declare active: string | null;
  declare lineReady: boolean;
  private off: (() => void)[] = [];

  constructor() {
    super();
    this.dark = false;
    this.plots = [];
    this.active = null;
    this.lineReady = false;
  }

  static styles = [
    base,
    css`
      :host {
        display: flex;
        flex-direction: column;
        height: 100%;
        min-height: 0;
      }
      .tabs {
        display: flex;
        gap: 2px;
        padding: 6px 8px 0;
        border-bottom: 1px solid var(--uc-border);
        flex-wrap: wrap;
      }
      .tabs button {
        border-radius: 6px 6px 0 0;
        border-bottom: none;
        background: transparent;
      }
      .tabs button[aria-selected='true'] {
        background: var(--uc-panel);
        border-color: var(--uc-border);
        font-weight: 600;
      }
      .body {
        flex: 1;
        min-height: 0;
        display: flex;
        flex-direction: column;
      }
      .empty {
        padding: 24px;
      }
      .stats {
        margin-left: auto;
        align-self: center;
        padding-bottom: 4px;
      }
    `,
  ];

  connectedCallback() {
    super.connectedCallback();
    this.off.push(this.client.on('state', (s) => this.onState(s)));
    if (this.client.decodeState) this.onState(this.client.decodeState);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this.off.forEach((f) => f());
    this.off = [];
  }

  private onState(s: DecodeState) {
    const names = s.plots.map((p) => `${p.name}:${p.kind}`).join('|');
    if (names !== this.plots.map((p) => `${p.name}:${p.kind}`).join('|')) this.plots = s.plots;
    else this.plots.splice(0, this.plots.length, ...s.plots);
    if (!this.active || !s.plots.some((p) => p.name === this.active)) this.active = s.plots[0]?.name ?? null;
    const a = s.plots.find((p) => p.name === this.active);
    if (a?.kind === 'line' && !this.lineReady) void import('./line-plot.ts').then(() => (this.lineReady = true));
  }

  render() {
    const s = this.client.decodeState;
    if (!this.plots.length) {
      return html`<div class="empty muted">
        No plots yet. The device creates them on port 60.
        ${s?.plotOrphans ? html`<br />${s.plotOrphans} frames for plots created before this connection were dropped.` : nothing}
      </div>`;
    }
    const a = this.plots.find((p) => p.name === this.active);
    return html`
      <div class="tabs" role="tablist" part="plot-tabs">
        ${this.plots.map(
          (p) =>
            html`<button role="tab" aria-selected=${p.name === this.active} @click=${() => (this.active = p.name)}>
              ${p.name}
            </button>`,
        )}
        <span class="stats muted">
          ${s ? `${s.plotFrames} frames` : ''}${s?.plotOrphans ? ` · ${s.plotOrphans} for unknown plots` : ''}${s?.plotErrors
            ? ` · ${s.plotErrors} invalid`
            : ''}
        </span>
      </div>
      <div class="body">
        ${!a
          ? nothing
          : a.kind === 'line'
            ? this.lineReady
              ? html`<uc-line-plot .client=${this.client} .name=${a.name} ?dark=${this.dark}></uc-line-plot>`
              : html`<div class="empty muted">Loading…</div>`
            : html`<uc-grid-plot .client=${this.client} .name=${a.name} ?dark=${this.dark}></uc-grid-plot>`}
      </div>
    `;
  }

  /** CSV of every plot (Export all). */
  async exportAll(): Promise<{ name: string; data: string }[]> {
    const out = [];
    for (const p of this.plots) out.push({ name: `plot-${safeName(p.name)}.csv`, data: await this.client.exportPlot(p.name) });
    return out;
  }
}

customElements.define('uc-plots', UcPlots);
