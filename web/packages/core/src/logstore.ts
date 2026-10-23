// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** Log store for the log view: a bounded ring of records, filters and the filtered index. */

import { LEVELS } from './logdata.ts';
import { levelIndex, type LogRecord } from './records.ts';

export interface Row extends LogRecord {
  lvl: number;
}

export interface FilterSpec {
  minLevel: number;
  /** Substring of the file name. */
  source: string;
  /** Exact line number. */
  line: string;
  /** Regular expression (case-insensitive) or empty. */
  search: string;
}

export const NO_FILTER: FilterSpec = { minLevel: 0, source: '', line: '', search: '' };

export class Filter {
  readonly re: RegExp | null;
  constructor(readonly spec: FilterSpec = NO_FILTER) {
    this.re = spec.search ? new RegExp(spec.search, 'i') : null; // throws on a bad pattern
  }
  matches(r: Row): boolean {
    // Error records are always shown (README §5.6).
    if (r.error) return !this.re || this.re.test(r.text);
    const s = this.spec;
    return (
      r.lvl >= s.minLevel &&
      (!s.source || r.file.includes(s.source)) &&
      (!s.line || r.line === s.line) &&
      (!this.re || this.re.test(r.text) || this.re.test(r.file))
    );
  }
  get active(): boolean {
    const s = this.spec;
    return s.minLevel > 0 || !!s.source || !!s.line || !!s.search;
  }
  describe(): string {
    const s = this.spec;
    const v = [`level ≥ ${LEVELS[s.minLevel]}`];
    if (s.source) v.push(`source ${s.source}`);
    if (s.line) v.push(`line ${s.line}`);
    if (s.search) v.push(`/${s.search}/`);
    return v.join(' · ');
  }
}

/** A FIFO with O(1) push/shift and indexed access. */
class Fifo<T> {
  private a: T[] = [];
  private h = 0;
  get length() {
    return this.a.length - this.h;
  }
  push(v: T) {
    this.a.push(v);
  }
  shift(): T | undefined {
    if (this.h >= this.a.length) return undefined;
    const v = this.a[this.h++];
    if (this.h > 4096 && this.h * 2 > this.a.length) {
      this.a = this.a.slice(this.h);
      this.h = 0;
    }
    return v;
  }
  at(i: number): T | undefined {
    return i >= 0 && i < this.length ? this.a[this.h + i] : undefined;
  }
  first(): T | undefined {
    return this.at(0);
  }
  clear() {
    this.a = [];
    this.h = 0;
  }
  replace(v: T[]) {
    this.a = v;
    this.h = 0;
  }
  *[Symbol.iterator]() {
    for (let i = this.h; i < this.a.length; i++) yield this.a[i];
  }
}

export class LogStore {
  private rows = new Fifo<Row>();
  private visible = new Fifo<number>();
  private nextSeq = 0;
  filter = new Filter();
  /** Drop rows that do not pass the filter instead of storing them. */
  discardFiltered = false;
  errorCounts = new Map<string, number>();
  received = 0;
  discarded = 0;

  constructor(private max = 100_000) {
    this.max = Math.max(100, max);
  }

  push(r: Omit<LogRecord, 'seq'>): Row | undefined {
    this.received++;
    if (r.error) this.errorCounts.set(r.error, (this.errorCounts.get(r.error) ?? 0) + 1);
    const row: Row = { ...r, seq: this.nextSeq, lvl: levelIndex(r.level) };
    const ok = this.filter.matches(row);
    if (!ok && this.discardFiltered) {
      this.discarded++;
      return undefined;
    }
    this.nextSeq++;
    if (ok) this.visible.push(row.seq);
    this.rows.push(row);
    while (this.rows.length > this.max) {
      const old = this.rows.shift()!;
      if (this.visible.first() === old.seq) this.visible.shift();
    }
    return row;
  }

  setFilter(f: Filter) {
    this.filter = f;
    this.refilter();
  }

  private refilter() {
    const v: number[] = [];
    for (const r of this.rows) if (this.filter.matches(r)) v.push(r.seq);
    this.visible.replace(v);
  }

  setMax(max: number) {
    this.max = Math.max(100, max);
    while (this.rows.length > this.max) this.rows.shift();
    this.refilter();
  }

  clear() {
    this.rows.clear();
    this.visible.clear();
  }

  get length() {
    return this.rows.length;
  }

  get visibleLength() {
    return this.visible.length;
  }

  /** The i-th visible row. */
  visibleAt(i: number): Row | undefined {
    const seq = this.visible.at(i);
    const first = this.rows.first();
    if (seq === undefined || !first) return undefined;
    return this.rows.at(seq - first.seq);
  }

  /** Visible rows [start, start+count). */
  window(start: number, count: number): Row[] {
    const out: Row[] = [];
    for (let i = start; i < Math.min(start + count, this.visibleLength); i++) out.push(this.visibleAt(i)!);
    return out;
  }

  get errors(): number {
    let n = 0;
    for (const v of this.errorCounts.values()) n += v;
    return n;
  }

  /** CSV of the visible rows (or of the given sequence numbers). */
  csv(only?: Set<number>): string {
    const q = (s: string) => `"${s.replace(/"/g, '""')}"`;
    let out = 'ts,level,file,line,text\n';
    for (let i = 0; i < this.visibleLength; i++) {
      const r = this.visibleAt(i)!;
      if (only && !only.has(r.seq)) continue;
      out += `${r.ts.toFixed(3)},${r.level},${q(r.file)},${q(r.line)},${q(r.text)}\n`;
    }
    return out;
  }
}

/** `ts:level:file:line:text`, the legacy `uclog-cli.py` line format. */
export function plainLine(r: LogRecord): string {
  return `${r.ts.toFixed(3)}:${r.level}:${r.file}:${r.line}:${r.text}`;
}
