// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** `<uc-grid-plot>`: a heatmap or spectrogram from port 60, drawn on a canvas with a colour bar. */

import { css, html, LitElement } from 'lit';
import type { Client, PlotData } from '@uclog/workers';
import { base } from './styles.ts';
import { exportPlot, PlotPoller } from './plot-poller.ts';

type GridData = Extract<PlotData, { kind: 'grid' }>;

// Viridis, 9 stops.
const STOPS = [
  [68, 1, 84],
  [71, 44, 122],
  [59, 81, 139],
  [44, 113, 142],
  [33, 144, 141],
  [39, 173, 129],
  [92, 200, 99],
  [170, 220, 50],
  [253, 231, 37],
];

export function colour(t: number): [number, number, number] {
  const x = Math.min(1, Math.max(0, t)) * (STOPS.length - 1);
  const i = Math.min(STOPS.length - 2, Math.floor(x));
  const f = x - i;
  const a = STOPS[i];
  const b = STOPS[i + 1];
  return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f];
}

const fmt = (v: number) => (Math.abs(v) >= 1e5 || (v !== 0 && Math.abs(v) < 1e-3) ? v.toExponential(2) : String(Number(v.toPrecision(4))));

export class UcGridPlot extends LitElement {
  static properties = {
    client: { attribute: false },
    name: { type: String },
    dark: { type: Boolean },
    info: { state: true },
    hover: { state: true },
  };
  declare client: Client;
  declare name: string;
  declare dark: boolean;
  declare info: { min: number; max: number; d: GridData } | null;
  declare hover: string;
  private poller: PlotPoller | null = null;
  private ro: ResizeObserver | null = null;

  constructor() {
    super();
    this.dark = false;
    this.info = null;
    this.hover = '';
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
      .bar {
        padding: 6px 8px;
      }
      .wrap {
        flex: 1;
        min-height: 0;
        display: grid;
        grid-template-columns: auto minmax(0, 1fr) auto;
        grid-template-rows: minmax(0, 1fr) auto;
        gap: 4px 8px;
        padding: 0 8px 8px;
      }
      .ylab {
        writing-mode: vertical-rl;
        transform: rotate(180deg);
        text-align: center;
      }
      canvas.map {
        width: 100%;
        height: 100%;
        min-height: 0;
        image-rendering: pixelated;
        border: 1px solid var(--uc-border);
        box-sizing: border-box;
      }
      .bar-scale {
        min-height: 0;
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 2px;
        font-size: 11px;
      }
      canvas.scale {
        width: 14px;
        flex: 1;
        min-height: 0;
      }
      .xlab {
        grid-column: 2;
        text-align: center;
        display: flex;
        justify-content: space-between;
      }
      .readout {
        margin-left: auto;
      }
    `,
  ];

  connectedCallback() {
    super.connectedCallback();
    this.poller = new PlotPoller(
      this.client,
      () => this.name,
      () => this.client.plot(this.name, null, null, 0),
      (d) => d.kind === 'grid' && this.draw(d),
    );
    this.poller.kick(true);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this.poller?.dispose();
    this.ro?.disconnect();
  }

  updated(ch: Map<string, unknown>) {
    if (ch.has('name') && ch.get('name') !== undefined) this.poller?.kick(true);
  }

  firstUpdated() {
    const scale = this.renderRoot.querySelector('canvas.scale') as HTMLCanvasElement;
    scale.width = 1;
    scale.height = 128;
    const ctx = scale.getContext('2d')!;
    for (let y = 0; y < 128; y++) {
      const [r, g, b] = colour(1 - y / 127);
      ctx.fillStyle = `rgb(${r},${g},${b})`;
      ctx.fillRect(0, y, 1, 1);
    }
  }

  private draw(d: GridData) {
    let min = Infinity;
    let max = -Infinity;
    for (const v of d.values) {
      if (Number.isNaN(v)) continue;
      if (v < min) min = v;
      if (v > max) max = v;
    }
    const c = this.renderRoot.querySelector('canvas.map') as HTMLCanvasElement;
    c.width = d.cols;
    c.height = d.rows;
    const ctx = c.getContext('2d')!;
    const img = ctx.createImageData(d.cols, d.rows);
    const span = max > min ? max - min : 1;
    for (let i = 0; i < d.values.length; i++) {
      const v = d.values[i];
      const o = i * 4;
      if (Number.isNaN(v)) {
        img.data[o + 3] = 0;
        continue;
      }
      const [r, g, b] = colour((v - min) / span);
      img.data[o] = r;
      img.data[o + 1] = g;
      img.data[o + 2] = b;
      img.data[o + 3] = 255;
    }
    ctx.putImageData(img, 0, 0);
    this.info = { min: Number.isFinite(min) ? min : 0, max: Number.isFinite(max) ? max : 0, d };
  }

  private move(e: MouseEvent) {
    const i = this.info;
    if (!i) return;
    const c = e.target as HTMLCanvasElement;
    const col = Math.floor((e.offsetX / c.clientWidth) * i.d.cols);
    const row = Math.floor((e.offsetY / c.clientHeight) * i.d.rows);
    const v = i.d.values[row * i.d.cols + col];
    const r = i.d.spectrogram ? i.d.rows - 1 - row : row;
    this.hover = v === undefined || Number.isNaN(v) ? '' : `row ${r} col ${col}: ${fmt(v)}`;
  }

  render() {
    const i = this.info;
    return html`
      <div class="bar row" part="toolbar">
        <button @click=${() => exportPlot(this.client, this.name)}>Export CSV</button>
        <span class="readout muted mono">${this.hover}</span>
      </div>
      <div class="wrap">
        <div class="ylab muted">${i?.d.ylabel ?? ''}</div>
        <canvas class="map" part="plot" @mousemove=${this.move} @mouseleave=${() => (this.hover = '')}></canvas>
        <div class="bar-scale muted">
          <span>${i ? fmt(i.max) : ''}</span>
          <canvas class="scale"></canvas>
          <span>${i ? fmt(i.min) : ''}</span>
        </div>
        <div class="xlab muted">
          ${i?.d.spectrogram ? html`<span>${fmt(i.d.x0)}</span>` : html`<span></span>`}
          <span>${i?.d.xlabel ?? ''}</span>
          ${i?.d.spectrogram ? html`<span>${fmt(i.d.x1)}</span>` : html`<span></span>`}
        </div>
      </div>
    `;
  }
}

customElements.define('uc-grid-plot', UcGridPlot);
