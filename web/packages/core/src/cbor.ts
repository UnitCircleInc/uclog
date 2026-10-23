// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Minimal CBOR (RFC 8949): enough for every ucLog payload (port 0, 59/3, 60, 62, logdata).
 *
 * Decoded maps are `Map`s, because logdata uses integer and array keys (protocol.md §6.1).
 * Integers beyond 2^53 decode as `bigint`. Encoding uses shortest heads, insertion-order map keys
 * and float64, like the reference (spec/vectors/README.md).
 */

export type CborValue =
  | number
  | bigint
  | string
  | boolean
  | null
  | undefined
  | Uint8Array
  | CborValue[]
  | Map<CborValue, CborValue>
  | CborFloat
  | { [k: string]: CborValue };

/** Forces a float64 encoding for a value that happens to be integral. */
export class CborFloat {
  constructor(readonly value: number) {}
}

export class CborError extends Error {}

const te = new TextEncoder();
const td = new TextDecoder('utf-8', { fatal: true });

class Writer {
  buf = new Uint8Array(256);
  n = 0;
  room(k: number) {
    if (this.n + k <= this.buf.length) return;
    const b = new Uint8Array(Math.max(this.buf.length * 2, this.n + k));
    b.set(this.buf.subarray(0, this.n));
    this.buf = b;
  }
  byte(b: number) {
    this.room(1);
    this.buf[this.n++] = b;
  }
  bytes(b: Uint8Array) {
    this.room(b.length);
    this.buf.set(b, this.n);
    this.n += b.length;
  }
  head(major: number, n: number | bigint) {
    const m = major << 5;
    if (typeof n === 'number' && n < 24) return this.byte(m | n);
    const big = BigInt(n);
    if (big < 0x100n) {
      this.byte(m | 24);
      this.byte(Number(big));
    } else if (big < 0x10000n) {
      this.byte(m | 25);
      this.byte(Number(big >> 8n));
      this.byte(Number(big & 0xffn));
    } else if (big < 0x100000000n) {
      this.byte(m | 26);
      const v = Number(big);
      this.bytes(new Uint8Array([v >>> 24, (v >>> 16) & 0xff, (v >>> 8) & 0xff, v & 0xff]));
    } else if (big < 0x10000000000000000n) {
      this.byte(m | 27);
      const d = new DataView(new ArrayBuffer(8));
      d.setBigUint64(0, big);
      this.bytes(new Uint8Array(d.buffer));
    } else {
      throw new CborError('integer too large for CBOR');
    }
  }
  float(v: number) {
    this.byte(0xfb);
    const d = new DataView(new ArrayBuffer(8));
    d.setFloat64(0, v);
    this.bytes(new Uint8Array(d.buffer));
  }
  value(v: CborValue) {
    if (v === null || v === undefined) return this.byte(0xf6);
    if (v === true) return this.byte(0xf5);
    if (v === false) return this.byte(0xf4);
    if (v instanceof CborFloat) return this.float(v.value);
    if (typeof v === 'number') {
      if (Number.isInteger(v) && Number.isSafeInteger(v)) {
        return v >= 0 ? this.head(0, v) : this.head(1, -1 - v);
      }
      return this.float(v);
    }
    if (typeof v === 'bigint') return v >= 0n ? this.head(0, v) : this.head(1, -1n - v);
    if (typeof v === 'string') {
      const b = te.encode(v);
      this.head(3, b.length);
      return this.bytes(b);
    }
    if (v instanceof Uint8Array) {
      this.head(2, v.length);
      return this.bytes(v);
    }
    if (Array.isArray(v)) {
      this.head(4, v.length);
      for (const x of v) this.value(x);
      return;
    }
    if (v instanceof Map) {
      this.head(5, v.size);
      for (const [k, x] of v) {
        this.value(k);
        this.value(x);
      }
      return;
    }
    const entries = Object.entries(v);
    this.head(5, entries.length);
    for (const [k, x] of entries) {
      this.value(k);
      this.value(x);
    }
  }
}

/** Encode one value. Plain objects become maps with text keys in insertion order. */
export function encode(v: CborValue): Uint8Array {
  const w = new Writer();
  w.value(v);
  return w.buf.slice(0, w.n);
}

class Reader {
  i = 0;
  dv: DataView;
  constructor(readonly d: Uint8Array) {
    this.dv = new DataView(d.buffer, d.byteOffset, d.byteLength);
  }
  need(k: number) {
    if (this.i + k > this.d.length) throw new CborError('truncated');
  }
  byte(): number {
    this.need(1);
    return this.d[this.i++];
  }
  peek(): number {
    this.need(1);
    return this.d[this.i];
  }
  item(): CborValue {
    const ib = this.byte();
    const major = ib >> 5;
    const ai = ib & 31;
    if (major === 7) {
      if (ai === 20) return false;
      if (ai === 21) return true;
      if (ai === 22 || ai === 23) return null;
      if (ai === 25) {
        this.need(2);
        const v = half(this.dv.getUint16(this.i));
        this.i += 2;
        return v;
      }
      if (ai === 26) {
        this.need(4);
        const v = this.dv.getFloat32(this.i);
        this.i += 4;
        return v;
      }
      if (ai === 27) {
        this.need(8);
        const v = this.dv.getFloat64(this.i);
        this.i += 8;
        return v;
      }
      throw new CborError(`unsupported simple value ${ai}`);
    }
    let n: number | bigint | null;
    if (ai < 24) n = ai;
    else if (ai === 24) n = this.byte();
    else if (ai === 25) {
      this.need(2);
      n = this.dv.getUint16(this.i);
      this.i += 2;
    } else if (ai === 26) {
      this.need(4);
      n = this.dv.getUint32(this.i);
      this.i += 4;
    } else if (ai === 27) {
      this.need(8);
      const b = this.dv.getBigUint64(this.i);
      this.i += 8;
      n = b <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(b) : b;
    } else if (ai === 31 && major >= 2 && major <= 5) n = null;
    else throw new CborError(`bad additional info ${ai}`);

    switch (major) {
      case 0:
        return n!;
      case 1:
        if (typeof n === 'bigint') return -1n - n;
        return n! < Number.MAX_SAFE_INTEGER ? -1 - (n as number) : -1n - BigInt(n!);
      case 2:
      case 3: {
        let raw: Uint8Array;
        if (n === null) {
          const parts: Uint8Array[] = [];
          while (this.peek() !== 0xff) {
            const p = this.item();
            parts.push(typeof p === 'string' ? te.encode(p) : (p as Uint8Array));
          }
          this.i++;
          raw = concat(parts);
        } else {
          const len = Number(n);
          this.need(len);
          raw = this.d.slice(this.i, this.i + len);
          this.i += len;
        }
        if (major === 2) return raw;
        try {
          return td.decode(raw);
        } catch (e) {
          throw new CborError(`invalid UTF-8 in text`);
        }
      }
      case 4: {
        const out: CborValue[] = [];
        if (n === null) {
          while (this.peek() !== 0xff) out.push(this.item());
          this.i++;
        } else {
          for (let k = 0; k < Number(n); k++) out.push(this.item());
        }
        return out;
      }
      case 5: {
        const out = new Map<CborValue, CborValue>();
        const one = () => {
          const k = this.item();
          out.set(k, this.item());
        };
        if (n === null) {
          while (this.peek() !== 0xff) one();
          this.i++;
        } else {
          for (let k = 0; k < Number(n); k++) one();
        }
        return out;
      }
      default:
        return this.item(); // major 6: tag, ignored
    }
  }
}

function half(h: number): number {
  const s = h & 0x8000 ? -1 : 1;
  const e = (h >> 10) & 0x1f;
  const f = h & 0x3ff;
  if (e === 0) return s * 2 ** -14 * (f / 1024);
  if (e === 31) return f ? NaN : s * Infinity;
  return s * 2 ** (e - 15) * (1 + f / 1024);
}

export function concat(parts: Uint8Array[]): Uint8Array {
  let n = 0;
  for (const p of parts) n += p.length;
  const out = new Uint8Array(n);
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

/** Decode exactly one item. */
export function decode(data: Uint8Array): CborValue {
  const r = new Reader(data);
  const v = r.item();
  if (r.i !== data.length) throw new CborError(`${data.length - r.i} trailing bytes`);
  return v;
}

// ---------------------------------------------------------------------------------------------
// Access helpers for decoded maps
// ---------------------------------------------------------------------------------------------

export type CborMap = Map<CborValue, CborValue>;

export function isMap(v: CborValue): v is CborMap {
  return v instanceof Map;
}

export function get(v: CborValue, key: string): CborValue {
  return v instanceof Map ? v.get(key) : undefined;
}

export function getText(v: CborValue, key: string): string | undefined {
  const x = get(v, key);
  return typeof x === 'string' ? x : undefined;
}

export function getInt(v: CborValue, key: string): number | undefined {
  const x = get(v, key);
  if (typeof x === 'number' && Number.isInteger(x)) return x;
  if (typeof x === 'bigint') return Number(x);
  return undefined;
}

export function getBytes(v: CborValue, key: string): Uint8Array | undefined {
  const x = get(v, key);
  return x instanceof Uint8Array ? x : undefined;
}

/** A short human-readable rendering (device info fields, command replies). */
export function display(v: CborValue): string {
  if (v === null || v === undefined) return 'null';
  if (v instanceof Uint8Array) return `h'${hex(v)}'`;
  if (v instanceof CborFloat) return String(v.value);
  if (typeof v === 'string') return JSON.stringify(v);
  if (typeof v === 'bigint') return v.toString();
  if (typeof v !== 'object') return String(v);
  if (Array.isArray(v)) return `[${v.map(display).join(', ')}]`;
  const entries = v instanceof Map ? [...v] : Object.entries(v);
  return `{${entries.map(([k, x]) => `${display(k as CborValue)}: ${display(x)}`).join(', ')}}`;
}

/** Convert a decoded value to plain JSON-friendly data (maps with text keys → objects). */
export function toPlain(v: CborValue): unknown {
  if (v instanceof Map) {
    const allText = [...v.keys()].every((k) => typeof k === 'string');
    if (allText) return Object.fromEntries([...v].map(([k, x]) => [k, toPlain(x)]));
    return [...v].map(([k, x]) => [toPlain(k), toPlain(x)]);
  }
  if (Array.isArray(v)) return v.map(toPlain);
  if (v instanceof Uint8Array) return hex(v);
  if (typeof v === 'bigint') return v.toString();
  if (v instanceof CborFloat) return v.value;
  return v;
}

export function hex(b: Uint8Array, max = Infinity): string {
  let s = '';
  const n = Math.min(b.length, max);
  for (let i = 0; i < n; i++) s += b[i].toString(16).padStart(2, '0');
  if (b.length > n) s += '…';
  return s;
}

export function fromHex(s: string): Uint8Array {
  const out = new Uint8Array(s.length >> 1);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.substr(i * 2, 2), 16);
  return out;
}
