// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * The decode/presentation worker's logic (plan 02 §3): logdata selection and decoding, the log
 * store with its filters, plot models. The main thread asks for windows of rows and plot
 * snapshots (decimated to the pixel width), and gets a state summary at most every 50 ms.
 */

import {
  decimate,
  DecodeSession,
  display,
  Filter,
  gridValues,
  LogStore,
  Resolver,
  toPlain,
  type CborValue,
  type LogdataCache,
  type Row,
} from '@uclog/core';
import type { DecodeState, DecodeToMain, DecoderToTransport, MainToDecode, PlotData, TransportToDecoder } from './protocol.ts';

type Post = (m: DecodeToMain, transfer?: Transferable[]) => void;

const PUBLISH_MS = 50;

export class DecodeHost {
  readonly store = new LogStore(100_000);
  readonly resolver: Resolver;
  readonly session: DecodeSession;
  private dirty = true;
  private emitLogs = false;
  private newRows: Row[] = [];
  private timers: ReturnType<typeof setInterval>[] = [];

  constructor(
    private post: Post,
    cache: LogdataCache | null,
  ) {
    this.resolver = new Resolver({}, cache);
    this.session = new DecodeSession(
      this.resolver,
      (r) => {
        const row = this.store.push(r);
        if (row && this.emitLogs) this.newRows.push(row);
        this.dirty = true;
      },
      (e) => {
        if (e.t === 'device-info') {
          const fields: [string, string][] =
            e.info instanceof Map
              ? [...e.info].map(([k, v]) => [typeof k === 'string' ? k : display(k), fieldText(v)])
              : [['info', display(e.info)]];
          this.post({ t: 'device-info', info: toPlain(e.info), fields, appHash: e.appHash });
        } else if (e.t === 'logdata') this.post({ t: 'logdata', appHash: e.appHash, source: e.source, targets: e.targets });
        else this.post({ t: 'note', text: e.text });
        this.dirty = true;
      },
    );
    this.timers.push(setInterval(() => this.session.tick(), 250));
    this.timers.push(setInterval(() => this.publish(), PUBLISH_MS));
  }

  dispose() {
    this.timers.forEach(clearInterval);
  }

  private publish() {
    if (this.newRows.length) {
      this.post({ t: 'logs', rows: this.newRows });
      this.newRows = [];
    }
    if (!this.dirty) return;
    this.dirty = false;
    this.post({ t: 'state', state: this.state() });
  }

  state(): DecodeState {
    const p = this.session.plots;
    return {
      received: this.store.received,
      stored: this.store.length,
      visible: this.store.visibleLength,
      discarded: this.store.discarded,
      errorCounts: Object.fromEntries(this.store.errorCounts),
      plots: p.plots.map((x) => ({
        name: x.name,
        kind: x.p.kind === 'line' ? 'line' : x.p.spectrogram ? 'spectrogram' : 'heatmap',
        version: x.version,
      })),
      plotFrames: p.frames,
      plotOrphans: p.orphans,
      plotErrors: p.errors,
      filter: this.store.filter.active ? this.store.filter.describe() : '',
    };
  }

  /** Batches from the transport worker. */
  onTransport(m: TransportToDecoder, ack: (a: DecoderToTransport) => void) {
    for (const item of m.items) {
      this.session.handle(item);
      if (item.t === 'plot' || item.t === 'start') this.dirty = true;
    }
    ack({ t: 'ack', id: m.id });
  }

  handle(m: MainToDecode) {
    const reply = (id: number, ok: boolean, value?: unknown, error?: string) => this.post({ t: 'reply', id, ok, value, error });
    switch (m.t) {
      case 'init':
        return;
      case 'config':
        this.resolver.cfg = m.resolver;
        this.store.setMax(m.maxLines);
        this.emitLogs = m.emitLogs;
        this.dirty = true;
        return;
      case 'load-logdata':
        try {
          const ld = this.session.loadManual(m.data, m.name);
          this.dirty = true;
          return reply(m.id, true, { target: ld.target, appHash: ld.appHash ? [...ld.appHash] : null });
        } catch (e) {
          return reply(m.id, false, undefined, (e as Error).message);
        }
      case 'filter':
        try {
          this.store.discardFiltered = m.discardFiltered;
          this.store.setFilter(new Filter(m.spec));
          this.dirty = true;
          return reply(m.id, true);
        } catch (e) {
          return reply(m.id, false, undefined, (e as Error).message);
        }
      case 'window':
        return reply(m.id, true, { start: m.start, rows: this.store.window(m.start, m.count), total: this.store.visibleLength });
      case 'clear':
        this.store.clear();
        this.dirty = true;
        return;
      case 'clear-plots':
        this.session.plots.clearAll();
        this.dirty = true;
        return;
      case 'plot': {
        const d = this.plotData(m.name, m.x0, m.x1, m.width);
        if (!d) return reply(m.id, false, undefined, `no plot ${m.name}`);
        return reply(m.id, true, d);
      }
      case 'export-logs':
        return reply(m.id, true, this.store.csv(m.only ? new Set(m.only) : undefined));
      case 'export-plot':
        return reply(m.id, true, this.session.plots.csv(m.name));
    }
  }

  plotData(name: string, x0: number | null, x1: number | null, width: number): PlotData | undefined {
    const p = this.session.plots.get(name);
    if (!p) return undefined;
    if (p.p.kind === 'grid') {
      const g = p.p;
      return {
        kind: 'grid',
        name,
        version: p.version,
        spectrogram: g.spectrogram,
        rows: g.rows,
        cols: g.cols,
        xlabel: g.xlabel,
        ylabel: g.ylabel,
        values: gridValues(g).slice(),
        x0: g.columns[0]?.[0] ?? 0,
        x1: g.columns[g.columns.length - 1]?.[0] ?? 0,
      };
    }
    const l = p.p;
    let xmin = Infinity;
    let xmax = -Infinity;
    for (const a of l.axes)
      for (const s of a.series)
        if (s.x.length) {
          xmin = Math.min(xmin, s.x.get(0));
          xmax = Math.max(xmax, s.x.last()!);
        }
    const lo = x0 ?? xmin;
    const hi = x1 ?? xmax;
    const buckets = Math.max(16, Math.round(width));
    return {
      kind: 'line',
      name,
      version: p.version,
      xlabel: l.xlabel,
      axes: l.axes.map((a) => ({ label: a.label, limits: a.limits })),
      series: l.axes.flatMap((a, ai) =>
        a.series.map((s) => {
          const xs = s.x.length === s.y.length ? s.x.view() : Float64Array.from(s.y.view(), (_, i) => i);
          const [x, y] = decimate(xs, s.y.view(), lo, hi, buckets);
          return { name: s.name, props: s.props, axis: ai, x, y };
        }),
      ),
      xmin: Number.isFinite(xmin) ? xmin : 0,
      xmax: Number.isFinite(xmax) ? xmax : 0,
    };
  }
}

function fieldText(v: CborValue): string {
  if (v instanceof Uint8Array) return [...v].map((b) => b.toString(16).padStart(2, '0')).join('');
  if (typeof v === 'string') return v;
  return display(v);
}
