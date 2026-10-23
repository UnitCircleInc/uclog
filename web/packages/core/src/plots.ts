// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Plot model (port 60, protocol.md §5.6): line plots with up to three y-axes, heatmaps and
 * spectrograms, as the legacy GUI drew them. No drawing here.
 */

import { decode, get, getInt, getText, type CborValue } from './cbor.ts';

/** Points kept per series when the plot sets no `xscroll`. */
export const SERIES_MAX = 1_000_000;

/** Growable float ring: append, drop from the front. */
export class Floats {
  buf = new Float64Array(64);
  start = 0;
  length = 0;
  push(v: number) {
    if (this.start + this.length === this.buf.length) {
      if (this.start > this.buf.length / 2) {
        this.buf.copyWithin(0, this.start, this.start + this.length);
      } else {
        const b = new Float64Array(this.buf.length * 2);
        b.set(this.buf.subarray(this.start, this.start + this.length));
        this.buf = b;
      }
      this.start = 0;
    }
    this.buf[this.start + this.length++] = v;
  }
  dropFront(n: number) {
    n = Math.min(n, this.length);
    this.start += n;
    this.length -= n;
  }
  get(i: number) {
    return this.buf[this.start + i];
  }
  last(): number | undefined {
    return this.length ? this.buf[this.start + this.length - 1] : undefined;
  }
  view(): Float64Array {
    return this.buf.subarray(this.start, this.start + this.length);
  }
  clear() {
    this.start = 0;
    this.length = 0;
  }
}

export interface Series {
  name: string;
  /** Renderer hints (`scatter`, `digital`, `infinite`). */
  props: string;
  x: Floats;
  y: Floats;
}

export function hasProp(s: { props: string }, p: string) {
  return s.props.split(/[^A-Za-z0-9]/).includes(p);
}

export interface YAxis {
  label: string;
  limits: [number, number] | null;
  series: Series[];
}

export interface LinePlot {
  kind: 'line';
  xlabel: string;
  axes: YAxis[];
  /** Points kept per series (0 = all, up to SERIES_MAX). */
  xscroll: number;
}

export interface GridPlot {
  kind: 'grid';
  spectrogram: boolean;
  rows: number;
  cols: number;
  xlabel: string;
  ylabel: string;
  /** Heatmap: the latest grid, row-major, top row first. */
  values: Float64Array;
  /** Spectrogram: the newest `cols` columns (x, one value per row, row 0 = lowest). */
  columns: [number, Float64Array][];
}

/** Row-major values for drawing, top row first; missing cells are NaN. */
export function gridValues(g: GridPlot): Float64Array {
  if (!g.spectrogram) return g.values;
  const out = new Float64Array(g.rows * g.cols).fill(NaN);
  const skip = g.cols - Math.min(g.columns.length, g.cols);
  g.columns.forEach(([, col], c) => {
    for (let r = 0; r < Math.min(col.length, g.rows); r++) out[(g.rows - 1 - r) * g.cols + skip + c] = col[r];
  });
  return out;
}

export interface Plot {
  name: string;
  p: LinePlot | GridPlot;
  /** Bumped on every change. */
  version: number;
}

function texts(v: CborValue): string[] {
  if (Array.isArray(v)) return v.map((x) => (typeof x === 'string' ? x : ''));
  return typeof v === 'string' ? [v] : [];
}

function num(v: CborValue): number | undefined {
  if (typeof v === 'number') return v;
  if (typeof v === 'bigint') return Number(v);
  if (typeof v === 'boolean') return v ? 1 : 0;
  return undefined;
}

function nums(v: CborValue): number[] {
  if (Array.isArray(v)) return v.map(num).filter((x): x is number => x !== undefined);
  const n = num(v);
  return n === undefined ? [] : [n];
}

function limits(v: CborValue): [number, number] | null {
  const l = nums(v);
  return l.length === 2 && l[1] > l[0] ? [l[0], l[1]] : null;
}

/** `yAS` → [axis 0–2, series 0–8]. */
function seriesKey(k: CborValue): [number, number] | null {
  return typeof k === 'string' && /^y[1-3][1-9]$/.test(k) ? [k.charCodeAt(1) - 49, k.charCodeAt(2) - 49] : null;
}

export class PlotSet {
  /** In creation order. */
  plots: Plot[] = [];
  frames = 0;
  /** Data for plots that do not exist (created before this client connected), counted. */
  orphans = 0;
  errors = 0;
  /** Plots created since the view last looked. */
  created: string[] = [];

  get(name: string): Plot | undefined {
    return this.plots.find((p) => p.name === name);
  }

  private put(name: string, p: LinePlot | GridPlot) {
    const old = this.get(name);
    if (old) {
      old.p = p;
      old.version++;
    } else {
      this.plots.push({ name, p, version: 0 });
      this.created.push(name);
    }
  }

  clearAll() {
    this.plots = [];
    this.created = [];
  }

  /** One port-60 payload. */
  handle(data: Uint8Array) {
    this.frames++;
    let m: CborValue;
    try {
      m = decode(data);
    } catch {
      this.errors++;
      return;
    }
    if (!(m instanceof Map)) {
      this.errors++;
      return;
    }
    this.handleMap(m);
  }

  private handleMap(m: Map<CborValue, CborValue>) {
    const has = (k: string) => m.has(k);
    const clear = getText(m, 'clear_plot');
    if (clear !== undefined) {
      const p = this.get(clear);
      if (!p) return void this.orphans++;
      if (p.p.kind === 'line') {
        for (const a of p.p.axes)
          for (const s of a.series) {
            s.x.clear();
            s.y.clear();
          }
      } else {
        p.p.values.fill(NaN);
        p.p.columns = [];
      }
      p.version++;
      return;
    }
    const name = getText(m, 'plot');
    if (name !== undefined) {
      if (has('xlabel') || has('yser')) {
        const ylabels = texts(get(m, 'ylabel'));
        const axes: YAxis[] = [];
        const keys: [string, string, string][] = [
          ['yser', 'ylimits', 'yprops'],
          ['y2ser', 'y2limits', 'y2props'],
          ['y3ser', 'y3limits', 'y3props'],
        ];
        for (let i = 0; i < 3; i++) {
          const [ser, lim, props] = keys[i];
          const names = texts(get(m, ser));
          if (!names.length) {
            if (i === 0) continue;
            break;
          }
          const pr = texts(get(m, props));
          axes.push({
            label: ylabels[i] ?? '',
            limits: limits(get(m, lim)),
            series: names.map((n, j) => ({ name: n, props: pr[j] ?? '', x: new Floats(), y: new Floats() })),
          });
        }
        this.put(name, {
          kind: 'line',
          xlabel: getText(m, 'xlabel') ?? '',
          axes,
          xscroll: Math.max(0, getInt(m, 'xscroll') ?? 0),
        });
        return;
      }
      const p = this.get(name);
      if (!p || p.p.kind !== 'line') return void this.orphans++;
      const l = p.p;
      const xs = nums(get(m, 'x'));
      const keep = l.xscroll > 0 ? Math.min(l.xscroll, SERIES_MAX) : SERIES_MAX;
      for (const [k, v] of m) {
        const sk = seriesKey(k);
        if (!sk) continue;
        const s = l.axes[sk[0]]?.series[sk[1]];
        if (!s) continue;
        const ys = nums(v);
        if (hasProp(s, 'infinite')) {
          for (const y of ys) s.y.push(y);
        } else {
          ys.forEach((y, i) => {
            const x = xs[i] ?? (s.x.length ? s.x.last()! + 1 : 0);
            s.x.push(x);
            s.y.push(y);
          });
        }
        if (s.y.length > keep) s.y.dropFront(s.y.length - keep);
        if (s.x.length > keep) s.x.dropFront(s.x.length - keep);
      }
      p.version++;
      return;
    }
    for (const [key, spectrogram] of [
      ['heatmap', false],
      ['spectrogram', true],
    ] as const) {
      const gname = getText(m, key);
      if (gname === undefined) continue;
      if (has('heatmap_rows') || has('xlabel')) {
        const rows = Math.min(Math.max(getInt(m, 'heatmap_rows') ?? 0, 0), 4096);
        const cols = Math.min(Math.max(getInt(m, 'heatmap_cols') ?? 0, 0), 4096);
        if (!rows || !cols) return void this.errors++;
        this.put(gname, {
          kind: 'grid',
          spectrogram,
          rows,
          cols,
          xlabel: getText(m, 'xlabel') ?? '',
          ylabel: texts(get(m, 'ylabel'))[0] ?? '',
          values: new Float64Array(rows * cols).fill(NaN),
          columns: [],
        });
        return;
      }
      const p = this.get(gname);
      if (!p || p.p.kind !== 'grid') return void this.orphans++;
      const g = p.p;
      const v = nums(get(m, 'y11'));
      if (g.spectrogram) {
        const x = nums(get(m, 'x'))[0] ?? g.columns.length;
        g.columns.push([x, Float64Array.from(v)]);
        if (g.columns.length > g.cols) g.columns.splice(0, g.columns.length - g.cols);
      } else if (v.length === g.rows * g.cols) {
        g.values = Float64Array.from(v);
      } else return void this.errors++;
      p.version++;
      return;
    }
    this.errors++;
  }

  /** CSV of one plot: `x,<series>…` when the series share x, else an `x_<name>,<name>` pair per
   * series. Grids: one row per row (heatmap) or column (spectrogram, with its x). */
  csv(name: string): string | undefined {
    const p = this.get(name);
    if (!p) return undefined;
    const q = (s: string) => (/[",]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s);
    const f = (v: number | undefined) => (v === undefined || Number.isNaN(v) ? '' : String(v));
    let out = '';
    if (p.p.kind === 'line') {
      const all = p.p.axes.flatMap((a) => a.series);
      const same = (a: Series, b: Series) => a.x.length === b.x.length && a.x.view().every((v, i) => v === b.x.get(i));
      const shared = all.every((s, i) => i === 0 || same(all[i - 1], s));
      const n = Math.max(0, ...all.map((s) => s.y.length));
      if (shared) {
        out += [q(p.p.xlabel || 'x'), ...all.map((s) => q(s.name))].join(',') + '\n';
        for (let i = 0; i < n; i++) {
          out += [f(all[0]?.x.length > i ? all[0].x.get(i) : undefined), ...all.map((s) => f(i < s.y.length ? s.y.get(i) : undefined))].join(',') + '\n';
        }
      } else {
        out += all.flatMap((s) => [q(`x_${s.name}`), q(s.name)]).join(',') + '\n';
        for (let i = 0; i < n; i++) {
          out +=
            all.flatMap((s) => [f(i < s.x.length ? s.x.get(i) : undefined), f(i < s.y.length ? s.y.get(i) : undefined)]).join(',') + '\n';
        }
      }
    } else {
      const g = p.p;
      if (g.spectrogram) {
        out += ['x', ...Array.from({ length: g.rows }, (_, r) => `row${r}`)].join(',') + '\n';
        for (const [x, col] of g.columns) out += [f(x), ...Array.from(col, f)].join(',') + '\n';
      } else {
        for (let r = 0; r < g.rows; r++) out += Array.from(g.values.subarray(r * g.cols, (r + 1) * g.cols), f).join(',') + '\n';
      }
    }
    return out;
  }
}

/**
 * Min/max decimation to about `buckets` buckets over [x0, x1], keeping the first and last point
 * (for drawing a window of a long series). Returns x and y arrays.
 */
export function decimate(x: Float64Array, y: Float64Array, x0: number, x1: number, buckets: number): [Float64Array, Float64Array] {
  // Visible range by binary search (x is assumed ascending; otherwise everything is used).
  let a = 0;
  let b = x.length;
  const asc = x.length < 2 || x[0] <= x[x.length - 1];
  if (asc) {
    let lo = 0,
      hi = x.length;
    while (lo < hi) {
      const m = (lo + hi) >> 1;
      if (x[m] < x0) lo = m + 1;
      else hi = m;
    }
    a = Math.max(0, lo - 1);
    lo = a;
    hi = x.length;
    while (lo < hi) {
      const m = (lo + hi) >> 1;
      if (x[m] <= x1) lo = m + 1;
      else hi = m;
    }
    b = Math.min(x.length, lo + 1);
  }
  const n = b - a;
  if (n <= buckets * 2) return [x.slice(a, b), y.slice(a, b)];
  const ox: number[] = [x[a]];
  const oy: number[] = [y[a]];
  const per = n / buckets;
  for (let k = 0; k < buckets; k++) {
    const s = a + Math.floor(k * per);
    const e = Math.min(b, a + Math.floor((k + 1) * per));
    let mi = s,
      ma = s;
    for (let i = s; i < e; i++) {
      if (y[i] < y[mi]) mi = i;
      if (y[i] > y[ma]) ma = i;
    }
    for (const i of mi === ma ? [mi] : mi < ma ? [mi, ma] : [ma, mi]) {
      ox.push(x[i]);
      oy.push(y[i]);
    }
  }
  ox.push(x[b - 1]);
  oy.push(y[b - 1]);
  return [Float64Array.from(ox), Float64Array.from(oy)];
}
