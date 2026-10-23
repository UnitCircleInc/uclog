// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * `<uc-line-plot>`: a port-60 line plot with up to three y-axes (uPlot scales `y`, `y2`, `y3`;
 * y2/y3 on the right). Follow mode fits all data; drag to zoom (follow stops), double-click to
 * go back. Click sets cursor A, Shift-click cursor B (Δx readout); Ctrl/⌘-click adds a marker.
 */

import { css, html, LitElement, nothing, unsafeCSS } from 'lit';
import uPlot from 'uplot';
import uplotCss from 'uplot/dist/uPlot.min.css?inline';
import { hasProp } from '@uclog/core';
import type { Client, PlotData } from '@uclog/workers';
import { base } from './styles.ts';
import { exportPlot, PlotPoller } from './plot-poller.ts';

type LineData = Extract<PlotData, { kind: 'line' }>;

const PALETTE = ['#4e79a7', '#f28e2b', '#59a14f', '#e15759', '#b07aa1', '#76b7b2', '#edc948', '#ff9da7', '#9c755f'];
const SCALES = ['y', 'y2', 'y3'];

function fmt(v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return '—';
  const a = Math.abs(v);
  return a !== 0 && (a < 1e-3 || a >= 1e6) ? v.toExponential(4) : String(Number(v.toPrecision(6)));
}

export class UcLinePlot extends LitElement {
  static properties = {
    client: { attribute: false },
    name: { type: String },
    dark: { type: Boolean },
    follow: { state: true },
    cursors: { state: true },
    markers: { state: true },
  };
  declare client: Client;
  declare name: string;
  declare dark: boolean;
  declare follow: boolean;
  declare cursors: [number | null, number | null];
  declare markers: number[];

  private u: uPlot | null = null;
  private shape = '';
  private poller: PlotPoller | null = null;
  private ro: ResizeObserver | null = null;
  private range: [number, number] | null = null;
  private last: LineData | null = null;

  constructor() {
    super();
    this.dark = false;
    this.follow = true;
    this.cursors = [null, null];
    this.markers = [];
  }

  static styles = [
    base,
    unsafeCSS(uplotCss),
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
      .chart {
        flex: 1;
        min-height: 0;
        padding: 0 8px;
      }
      .readout {
        margin-left: auto;
      }
      .u-legend {
        font-size: 12px;
        color: var(--uc-fg);
      }
      .u-select {
        background: rgb(127 127 127 / 0.15);
      }
    `,
  ];

  connectedCallback() {
    super.connectedCallback();
    this.poller = new PlotPoller(
      this.client,
      () => this.name,
      () => this.client.plot(this.name, this.follow ? null : (this.range?.[0] ?? null), this.follow ? null : (this.range?.[1] ?? null), this.width()),
      (d) => d.kind === 'line' && this.draw(d),
    );
    this.poller.kick(true);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this.poller?.dispose();
    this.ro?.disconnect();
    this.u?.destroy();
    this.u = null;
    this.shape = '';
  }

  updated(ch: Map<string, unknown>) {
    if (ch.has('name') && ch.get('name') !== undefined) {
      this.u?.destroy();
      this.u = null;
      this.shape = '';
      this.follow = true;
      this.cursors = [null, null];
      this.markers = [];
      this.poller?.kick(true);
    }
    if (ch.has('dark') && ch.get('dark') !== undefined && this.last) {
      this.u?.destroy();
      this.u = null;
      this.shape = '';
      this.draw(this.last);
    }
  }

  private width() {
    return (this.renderRoot.querySelector('.chart') as HTMLElement | null)?.clientWidth || 800;
  }

  private size() {
    const el = this.renderRoot.querySelector('.chart') as HTMLElement;
    return { width: Math.max(200, el.clientWidth - 16), height: Math.max(150, el.clientHeight - 60) };
  }

  /** uPlot wants one x array: use it when the series share x, else join them. */
  private table(d: LineData): uPlot.AlignedData {
    const s = d.series;
    const same = s.every((x) => x.x.length === s[0].x.length && x.x.every((v, i) => v === s[0].x[i]));
    if (same) return [s[0]?.x ?? new Float64Array(), ...s.map((x) => x.y)] as uPlot.AlignedData;
    return uPlot.join(s.map((x) => [x.x, x.y] as uPlot.AlignedData));
  }

  private draw(d: LineData) {
    this.last = d;
    const shape = `${d.series.map((s) => `${s.name}/${s.axis}/${s.props}`).join('|')}#${d.axes.length}`;
    if (!this.u || shape !== this.shape) this.create(d, shape);
    this.u!.setData(this.table(d), this.follow);
    if (!this.follow && this.range) this.u!.setScale('x', { min: this.range[0], max: this.range[1] });
  }

  private create(d: LineData, shape: string) {
    this.u?.destroy();
    this.shape = shape;
    const el = this.renderRoot.querySelector('.chart') as HTMLElement;
    const fg = this.dark ? '#c9ccd3' : '#333';
    const grid = this.dark ? 'rgb(255 255 255 / 0.08)' : 'rgb(0 0 0 / 0.08)';
    const scales: uPlot.Scales = { x: { time: false } };
    d.axes.forEach((a, i) => {
      scales[SCALES[i]] = a.limits ? { range: [a.limits[0], a.limits[1]] } : { auto: true };
    });
    const axes: uPlot.Axis[] = [
      { stroke: fg, grid: { stroke: grid }, ticks: { stroke: grid }, label: d.xlabel },
      ...d.axes.map(
        (a, i): uPlot.Axis => ({
          scale: SCALES[i],
          side: i === 0 ? 3 : 1,
          stroke: fg,
          label: a.label,
          grid: { show: i === 0, stroke: grid },
          ticks: { stroke: grid },
          size: 60,
        }),
      ),
    ];
    const series: uPlot.Series[] = [
      { label: d.xlabel || 'x', value: (_u, v) => fmt(v) },
      ...d.series.map(
        (s, i): uPlot.Series => ({
          label: d.axes.length > 1 ? `${s.name} (y${s.axis + 1})` : s.name,
          scale: SCALES[s.axis],
          stroke: PALETTE[i % PALETTE.length],
          width: 1.5,
          spanGaps: true,
          points: { show: hasProp(s, 'scatter'), size: 4 },
          paths: hasProp(s, 'scatter') ? () => null : hasProp(s, 'digital') ? uPlot.paths.stepped!({ align: 1 }) : undefined,
          value: (_u, v) => fmt(v),
        }),
      ),
    ];
    const self = this;
    this.u = new uPlot(
      {
        ...this.size(),
        scales,
        axes,
        series,
        legend: { live: true },
        cursor: { drag: { x: true, y: false, setScale: false }, focus: { prox: 16 } },
        hooks: {
          // A drag selects an x range: zoom to it (follow stops).
          setSelect: [
            (u) => {
              if (u.select.width < 3) return;
              const min = u.posToVal(u.select.left, 'x');
              const max = u.posToVal(u.select.left + u.select.width, 'x');
              u.setSelect({ left: 0, top: 0, width: 0, height: 0 }, false);
              self.range = [min, max];
              self.follow = false;
              u.setScale('x', { min, max });
              self.poller?.kick(true);
            },
          ],
          draw: [(u) => self.drawCursors(u)],
          ready: [
            (u) => {
              u.over.addEventListener('dblclick', () => {
                self.follow = true;
                self.range = null;
                self.poller?.kick(true);
              });
              let down = 0;
              u.over.addEventListener('mousedown', (e: MouseEvent) => (down = e.clientX));
              u.over.addEventListener('click', (e: MouseEvent) => {
                if (Math.abs(e.clientX - down) > 2) return; // end of a drag
                const x = u.posToVal(e.offsetX, 'x');
                if (e.metaKey || e.ctrlKey) self.markers = [...self.markers, x];
                else if (e.shiftKey) self.cursors = [self.cursors[0], x];
                else self.cursors = [x, self.cursors[1]];
                u.redraw(false);
              });
            },
          ],
        },
      },
      this.table(d),
      el,
    );
    this.ro?.disconnect();
    this.ro = new ResizeObserver(() => this.u?.setSize(this.size()));
    this.ro.observe(el);
  }

  private drawCursors(u: uPlot) {
    const ctx = u.ctx;
    const { top, height } = u.bbox;
    const line = (x: number, colour: string, label: string) => {
      const px = Math.round(u.valToPos(x, 'x', true));
      if (px < u.bbox.left || px > u.bbox.left + u.bbox.width) return;
      ctx.save();
      ctx.strokeStyle = colour;
      ctx.setLineDash([4, 3]);
      ctx.lineWidth = devicePixelRatio;
      ctx.beginPath();
      ctx.moveTo(px, top);
      ctx.lineTo(px, top + height);
      ctx.stroke();
      ctx.fillStyle = colour;
      ctx.font = `${11 * devicePixelRatio}px system-ui`;
      ctx.fillText(label, px + 3 * devicePixelRatio, top + 12 * devicePixelRatio);
      ctx.restore();
    };
    const [a, b] = this.cursors;
    if (a !== null) line(a, '#e15759', 'A');
    if (b !== null) line(b, '#4e79a7', 'B');
    this.markers.forEach((m, i) => line(m, '#b07aa1', `M${i + 1}`));
  }

  render() {
    const [a, b] = this.cursors;
    const dx = a !== null && b !== null ? b - a : null;
    return html`
      <div class="bar row" part="toolbar">
        <label title="Fit all data as it arrives; drag on the plot to zoom, double-click to fit again">
          <input
            type="checkbox"
            .checked=${this.follow}
            @change=${(e: Event) => {
              this.follow = (e.target as HTMLInputElement).checked;
              if (this.follow) this.range = null;
              this.poller?.kick(true);
            }}
          />
          Follow
        </label>
        <button @click=${() => exportPlot(this.client, this.name)}>Export CSV</button>
        ${this.markers.length || a !== null || b !== null
          ? html`<button
              @click=${() => {
                this.cursors = [null, null];
                this.markers = [];
                this.u?.redraw(false);
              }}
            >
              Clear cursors
            </button>`
          : nothing}
        <span class="readout muted mono">
          ${a !== null ? `A=${fmt(a)}` : 'click: cursor A, Shift-click: B, Ctrl/⌘-click: marker'}
          ${b !== null ? ` B=${fmt(b)}` : ''}
          ${dx !== null ? ` Δx=${fmt(dx)}${dx !== 0 ? ` 1/Δx=${fmt(1 / dx)}` : ''}` : ''}
        </span>
      </div>
      <div class="chart" part="plot"></div>
    `;
  }
}

customElements.define('uc-line-plot', UcLinePlot);
