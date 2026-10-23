// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * `loopback:` — a simulated device, the same as the native one (`uclog-link` loopback.rs), for
 * the demo page and tests without hardware. It logs with the formats of
 * `spec/vectors/sample.logdata` (target digit 1); its device info carries `LOOPBACK_APP_HASH`.
 *
 * Commands, as text lines ending in `\r` on port 61 or port 0: help, echo <text>, reset,
 * burst <n>, big, quiet <secs>, sh, read, plot [stop].
 */

import { CborFloat, encode, type CborValue } from './cbor.ts';
import { cobsDecode, FrameScanner, MAX_PORT_DATA, muxDecode, PORTS, wire, wirePort } from './frame.ts';
import { OP } from './semihost.ts';

export const LOOPBACK_APP_HASH = new Uint8Array(64).fill(0x5a);

const FMT_HELLO = 0xf0100000; // "Hello World! %s"
const FMT_TEMP = 0xf0100040; // "temp=%d mV=%u flags=0x%08x"
const FMT_NO_ARGS = 0xf0100280; // "no args"

export interface LoopbackOptions {
  /** Logs per second (0 = only replies and pulses). */
  rate: number;
  board: string;
}

export function parseLoopbackOptions(s: string): LoopbackOptions {
  const o: LoopbackOptions = { rate: 2, board: 'loopback' };
  for (const kv of s.split(',').filter(Boolean)) {
    const [k, v] = kv.split('=');
    if (v === undefined) throw new Error(`expected key=value, got ${JSON.stringify(kv)}`);
    if (k === 'rate') {
      o.rate = Number(v);
      if (!Number.isFinite(o.rate) || o.rate < 0) throw new Error(`bad rate ${JSON.stringify(v)}`);
    } else if (k === 'board') o.board = v;
    else throw new Error(`unknown option ${JSON.stringify(k)} (rate, board)`);
  }
  return o;
}

const te = new TextEncoder();
const u32 = (v: number) => {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setUint32(0, v >>> 0, true);
  return b;
};
const cat = (...p: Uint8Array[]) => {
  const out = new Uint8Array(p.reduce((n, x) => n + x.length, 0));
  let o = 0;
  for (const x of p) {
    out.set(x, o);
    o += x.length;
  }
  return out;
};

function log(addr: number, args: Uint8Array = new Uint8Array(0)) {
  return wire(cat(u32(addr), args));
}

function deviceInfo(board: string) {
  return wirePort(PORTS.DEVICE_INFO, encode({ app_hash: LOOPBACK_APP_HASH, board, image_type: 'app' }));
}

const floats = (v: number[]) => v.map((x) => new CborFloat(x));

function plotCreate(): CborValue[] {
  return [
    {
      plot: 'loopback',
      xlabel: 't (s)',
      ylabel: ['value', 'ramp (%)'],
      yser: ['sin', 'cos'],
      y2ser: ['ramp'],
      y2limits: floats([0, 100]),
      xscroll: 400,
    },
    { spectrogram: 'spectrum', heatmap_rows: 32, heatmap_cols: 100, xlabel: 't (s)', ylabel: ['bin'], yser: ['power'] },
    { heatmap: 'grid', heatmap_rows: 8, heatmap_cols: 8, xlabel: 'column', ylabel: ['row'], yser: ['level'] },
  ];
}

function plotData(n: number): CborValue[] {
  const t = Array.from({ length: 5 }, (_, i) => (n * 5 + i) * 0.01);
  const out: CborValue[] = [
    {
      plot: 'loopback',
      x: floats(t),
      y11: floats(t.map((x) => Math.sin(x * 6))),
      y12: floats(t.map((x) => Math.cos(x * 6) * 0.5)),
      y21: floats(t.map((x) => (x * 20) % 100)),
    },
  ];
  const ts = n * 0.05;
  const peak = 16 + 12 * Math.sin(ts * 0.8);
  out.push({
    spectrogram: 'spectrum',
    x: new CborFloat(ts),
    y11: floats(Array.from({ length: 32 }, (_, r) => Math.exp(-(((r - peak) / 3) ** 2)) * 100)),
  });
  if (n % 20 === 0) {
    const k = n / 20;
    out.push({
      heatmap: 'grid',
      x: new CborFloat(ts),
      y11: floats(Array.from({ length: 64 }, (_, i) => Math.sin((Math.floor(i / 8) + k) * 0.7) + Math.cos((i % 8) * 0.9))),
    });
  }
  return out;
}

type Pending = { kind: 'timeofday' | 'write' | 'read'; port: number; at: number };

/** The simulated device. Bytes it sends go to `out`; `write` takes host bytes. */
export class LoopbackDevice {
  private seq = 0;
  private quietUntil = 0;
  private lastTx = 0;
  private pending: Pending | null = null;
  private line: number[] = [];
  private plot: number | null = null;
  private scanner = new FrameScanner();
  private timers: ReturnType<typeof setInterval>[] = [];
  private closed = false;

  constructor(
    readonly o: LoopbackOptions,
    private out: (b: Uint8Array) => void,
  ) {}

  start() {
    this.send(deviceInfo(this.o.board));
    this.send(log(FMT_HELLO, te.encode('loopback start\0')));
    this.timers.push(setInterval(() => this.tick(), 50));
    if (this.o.rate > 0) this.timers.push(setInterval(() => this.oneLog(), 1000 / this.o.rate));
  }

  close() {
    this.closed = true;
    this.timers.forEach(clearInterval);
  }

  private send(b: Uint8Array) {
    if (this.closed || performance.now() < this.quietUntil) return;
    this.lastTx = performance.now();
    this.out(b);
  }

  private text(port: number, s: string) {
    this.send(wirePort(port, te.encode(s)));
  }

  private oneLog() {
    this.seq = (this.seq + 1) >>> 0;
    const s = this.seq;
    if (s % 3 === 0) this.send(log(FMT_HELLO, te.encode(`loopback #${s}\0`)));
    else if (s % 3 === 1) this.send(log(FMT_TEMP, cat(u32(s * 7 - 20), u32(3300 + (s % 100)), u32(s))));
    else this.send(log(FMT_NO_ARGS));
  }

  private tick() {
    if (this.plot !== null) {
      const n = this.plot++;
      for (const m of plotData(n)) this.send(wirePort(PORTS.PLOT, encode(m)));
    }
    const now = performance.now();
    if (this.pending && now - this.pending.at > 5000) {
      if (this.pending.kind !== 'read' || now - this.pending.at > 60_000) {
        const port = this.pending.port;
        this.pending = null;
        this.text(port, 'sh: no reply within the timeout\r\n');
      }
    }
    if (now - this.lastTx >= 500) this.send(new Uint8Array([0]));
  }

  /** Bytes from the host. */
  write(d: Uint8Array) {
    for (const e of this.scanner.feed(d)) if ('frame' in e) this.onFrame(e.frame);
  }

  private onFrame(enc: Uint8Array) {
    let p: Uint8Array;
    try {
      p = cobsDecode(enc);
    } catch {
      return;
    }
    const m = muxDecode(p);
    if (m.kind !== 'port') return;
    if (m.port === PORTS.SEMIHOST) return this.onSemihost(m.data);
    if (m.port === PORTS.DEVICE_INFO && m.data.length === 0) return this.send(deviceInfo(this.o.board));
    if (m.port === 0 || m.port === PORTS.TERMINAL) {
      for (const b of m.data) {
        if (b === 13 || b === 10) {
          const line = new TextDecoder().decode(new Uint8Array(this.line)).trim();
          this.line = [];
          if (line) this.command(m.port, line);
        } else this.line.push(b);
      }
    }
  }

  private command(port: number, line: string) {
    const sp = line.indexOf(' ');
    const [cmd, arg] = sp < 0 ? [line, ''] : [line.slice(0, sp), line.slice(sp + 1)];
    switch (cmd) {
      case 'help':
        return this.text(port, 'loopback commands: help, echo <text>, reset, burst <n>, big, quiet <secs>, sh, read, plot [stop]\r\n');
      case 'echo':
        return this.text(port, `${arg}\r\n`);
      case 'reset':
        this.seq = 0;
        this.send(deviceInfo(this.o.board));
        return this.send(log(FMT_HELLO, te.encode('loopback reset\0')));
      case 'burst': {
        const n = parseInt(arg, 10) || 100;
        for (let i = 0; i < n; i++) this.oneLog();
        return this.text(port, `burst of ${n} logs\r\n`);
      }
      case 'big': {
        const d = new Uint8Array(MAX_PORT_DATA).map((_, i) => 97 + (i % 26));
        d.set([13, 10], d.length - 2);
        return this.send(wirePort(port, d));
      }
      case 'quiet': {
        const s = Number(arg) || 3;
        this.text(port, `quiet for ${s} s\r\n`);
        this.quietUntil = performance.now() + s * 1000;
        return;
      }
      case 'sh':
        this.pending = { kind: 'timeofday', port, at: performance.now() };
        return this.send(wirePort(PORTS.SEMIHOST, new Uint8Array([OP.TIMEOFDAY])));
      case 'plot':
        if (arg === 'stop') {
          this.plot = null;
          return this.text(port, 'plots stopped\r\n');
        }
        for (const m of plotCreate()) this.send(wirePort(PORTS.PLOT, encode(m)));
        this.plot = 0;
        return this.text(port, 'plots: loopback (lines), spectrum, grid; `plot stop` to stop\r\n');
      case 'read':
        this.pending = { kind: 'read', port, at: performance.now() };
        return this.send(wirePort(PORTS.SEMIHOST, cat(new Uint8Array([OP.READ]), u32(0), u32(64))));
      default:
        return this.text(port, `unknown command ${JSON.stringify(cmd)} (try help)\r\n`);
    }
  }

  private onSemihost(d: Uint8Array) {
    const p = this.pending;
    if (!p) return;
    this.pending = null;
    const dv = new DataView(d.buffer, d.byteOffset, d.byteLength);
    const i32 = (o: number) => (d.length >= o + 4 ? dv.getInt32(o, true) : null);
    if (p.kind === 'timeofday') {
      const sec = d.length >= 8 ? dv.getBigUint64(0, true) : null;
      this.text(p.port, `sh: TIMEOFDAY sec=${sec === null ? 'None' : `Some(${sec})`} (${d.length} bytes)\r\n`);
      this.pending = { kind: 'write', port: p.port, at: performance.now() };
      return this.send(wirePort(PORTS.SEMIHOST, cat(new Uint8Array([OP.WRITE]), u32(1), te.encode('hello from loopback\n'))));
    }
    const fmt = (v: number | null) => (v === null ? 'None' : `Some(${v})`);
    if (p.kind === 'write') return this.text(p.port, `sh: WRITE r=${fmt(i32(0))} errno=${fmt(i32(4))}\r\n`);
    const text = new TextDecoder().decode(d.subarray(8));
    this.text(p.port, `read: n=${i32(0) ?? -1} errno=${fmt(i32(4))} ${JSON.stringify(text)}\r\n`);
  }
}
