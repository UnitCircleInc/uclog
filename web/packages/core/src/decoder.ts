// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * The decode half of a session (plan 02 §3): logdata selection by app_hash, log decoding, error
 * records, plots. Fed by the transport half's `ToDecoder` messages, in order.
 */

import { decode, getBytes, hex, type CborValue } from './cbor.ts';
import { Decoders, DecodeError, LogData } from './logdata.ts';
import { PlotSet } from './plots.ts';
import { UCLOG_SOURCE, type LogRecord } from './records.ts';
import { Resolver, type Resolved } from './resolver.ts';
import type { ToDecoder } from './transport.ts';

/** Log frames held while logdata is being resolved, at most. */
export const PENDING_MAX = 5000;
/** How long log frames are held after asking for the device info (F7), in ms. */
export const INFO_WAIT = 1000;

export type DecoderEvent =
  | { t: 'device-info'; info: CborValue; appHash: string | null }
  | { t: 'logdata'; appHash: string; source: Resolved['source']; targets: number[] }
  | { t: 'note'; text: string };

type LogMsg = Extract<ToDecoder, { t: 'log' }>;

export class DecodeSession {
  readonly decoders = new Decoders();
  readonly plots = new PlotSet();
  private pending: LogMsg[] | null = null;
  private infoWait: number | null = null;
  appHash: string | null = null;
  private appHashBytes: Uint8Array | null = null;

  constructor(
    readonly resolver: Resolver,
    private emitRecord: (r: Omit<LogRecord, 'seq'>) => void,
    private emit: (e: DecoderEvent) => void,
    private now: () => number = () => performance.now(),
  ) {}

  /** A new connection: forget the previous device. Manual logdata stays. */
  reset() {
    this.pending = null;
    this.infoWait = null;
    this.appHash = null;
    this.appHashBytes = null;
    this.decoders.clear();
    for (const ld of this.resolver.manual) this.decoders.insert(ld);
  }

  /** Load a logdata file manually. Returns its target digit. */
  loadManual(data: Uint8Array, name: string): LogData {
    const ld = LogData.fromFile(data, name);
    this.resolver.manual = this.resolver.manual.filter((x) => x.source !== name && x.target !== ld.target);
    this.resolver.manual.push(ld);
    this.decoders.insert(ld);
    // Remember it for this device, so the next connection finds it (replaces the legacy
    // target_info_archive).
    if (this.appHashBytes && (!ld.appHash || hex(ld.appHash) === this.appHash)) {
      void this.resolver.remember(this.appHashBytes, ld.cbor).catch(() => {});
    }
    return ld;
  }

  /** Call periodically (every 250 ms): ends the F7 hold for firmware that does not answer. */
  tick() {
    if (this.infoWait !== null && this.now() >= this.infoWait) {
      this.infoWait = null;
      if (this.appHash === null) this.flush();
    }
  }

  private flush() {
    const p = this.pending;
    this.pending = null;
    for (const m of p ?? []) this.decodeLog(m);
  }

  private error(ts: number, kind: string, text: string, target: number | null) {
    this.emitRecord({ ts, target, level: 'ERROR', file: UCLOG_SOURCE, line: '', text, error: kind });
  }

  handle(m: ToDecoder) {
    switch (m.t) {
      case 'start':
        this.reset();
        // Direct links ask for the device info; hold logs briefly so they decode with the
        // logdata it selects.
        if (m.direct && this.decoders.size === 0) {
          this.pending = [];
          this.infoWait = this.now() + INFO_WAIT;
        }
        return;
      case 'error':
        return this.error(m.ts, m.kind, m.text, m.target);
      case 'log':
        if (this.pending && this.pending.length < PENDING_MAX) {
          this.pending.push(m);
          return;
        }
        return this.decodeLog(m);
      case 'plot':
        return this.plots.handle(m.data);
      case 'info':
        return this.onInfo(m.ts, m.data);
    }
  }

  private decodeLog(m: LogMsg) {
    const ld = this.decoders.get(m.target);
    if (!ld) {
      return this.error(
        m.ts,
        'no_logdata',
        `decode: no logdata for target ${m.target} (addr 0x${m.addr.toString(16).padStart(8, '0')}) [${hex(m.args, 32)}]`,
        m.target,
      );
    }
    try {
      const d = ld.decode(m.addr, m.args);
      this.emitRecord({ ts: m.ts, target: m.target, level: d.level, file: d.file, line: d.line, text: d.text });
    } catch (e) {
      const de = e instanceof DecodeError ? e : new DecodeError('format', String(e));
      const text =
        de.kind === 'unknown_format'
          ? `decode: unknown format addr 0x${((m.addr & ~3) >>> 0).toString(16).padStart(8, '0')} in ${ld.source} [${hex(m.args, 32)}]`
          : `decode: ${de.message}`;
      this.error(m.ts, de.kind, text, m.target);
    }
  }

  private onInfo(ts: number, data: Uint8Array) {
    let info: CborValue;
    try {
      info = decode(data);
    } catch (e) {
      return this.error(ts, 'bad_cbor', `port 62: invalid CBOR (${(e as Error).message}) [${hex(data, 32)}]`, null);
    }
    const hash = getBytes(info, 'app_hash');
    this.emit({ t: 'device-info', info, appHash: hash ? hex(hash) : null });
    if (!hash) return this.emit({ t: 'note', text: 'device info has no app_hash' });
    const h = hex(hash);
    if (this.appHash === h && this.decoders.size) return;
    this.appHash = h;
    this.appHashBytes = hash;
    this.infoWait = null;
    this.pending ??= [];
    void this.resolver.resolve(hash).then(
      (r) => this.onResolved(h, r),
      (e) => this.onResolved(h, { logdata: [], source: null, notes: [String(e)] }),
    );
  }

  private onResolved(h: string, r: Resolved) {
    if (this.appHash !== h) return; // superseded by a newer device info
    for (const n of r.notes) this.emit({ t: 'note', text: n });
    if (r.source) {
      this.decoders.clear();
      for (const ld of r.logdata) this.decoders.insert(ld);
      // A manual file that matched: remember it for the next connection.
      if (r.source.kind === 'manual' && this.appHashBytes) {
        void this.resolver.remember(this.appHashBytes, r.logdata[0].cbor).catch(() => {});
      }
    }
    this.emit({ t: 'logdata', appHash: h, source: r.source, targets: r.source ? r.logdata.map((l) => l.target) : [] });
    this.flush();
  }
}
