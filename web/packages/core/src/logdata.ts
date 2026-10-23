// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** Logdata (`.logdata` CBOR, or an ELF's `.logdata_cbor` section) and log decoding (§6). */

import { decode as cborDecode, hex, type CborValue } from './cbor.ts';
import { TARGET_DIGIT_SHIFT, TYPE_MEM } from './frame.ts';
import { escapeStr, formatC, IntArg, type PrintfArg } from './printf.ts';

export const LEVELS = ['DEBUG', 'INFO', 'WARN', 'ERROR', 'FATAL', 'PANIC'] as const;

export class LogDataError extends Error {}

/** A log frame that could not be decoded; `kind` is the error-record kind (§7). */
export class DecodeError extends Error {
  constructor(
    readonly kind: string,
    message: string,
  ) {
    super(message);
  }
}

export interface Decoded {
  level: string;
  file: string;
  line: string;
  text: string;
}

/** Contents of section `name` of an ELF32/ELF64 file (either byte order), or undefined. */
export function elfSection(data: Uint8Array, name: string): Uint8Array | undefined {
  if (data.length < 0x34 || data[0] !== 0x7f || data[1] !== 0x45 || data[2] !== 0x4c || data[3] !== 0x46) {
    throw new LogDataError('not an ELF file');
  }
  const is64 = data[4] === 2;
  const le = data[5] === 1;
  const dv = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const u16 = (o: number) => dv.getUint16(o, le);
  const u32 = (o: number) => dv.getUint32(o, le);
  const u64 = (o: number) => Number(dv.getBigUint64(o, le));
  try {
    const shoff = is64 ? u64(0x28) : u32(0x20);
    const [shentsize, shnum, shstrndx] = is64 ? [u16(0x3a), u16(0x3c), u16(0x3e)] : [u16(0x2e), u16(0x30), u16(0x32)];
    const section = (i: number): [number, number, number] => {
      const o = shoff + i * shentsize;
      return is64 ? [u32(o), u64(o + 0x18), u64(o + 0x20)] : [u32(o), u32(o + 0x10), u32(o + 0x14)];
    };
    const [, stroff, strsize] = section(shstrndx);
    const names = data.subarray(stroff, stroff + strsize);
    const want = new TextEncoder().encode(name);
    for (let i = 0; i < shnum; i++) {
      const [nm, off, size] = section(i);
      let ok = names[nm + want.length] === 0;
      for (let k = 0; ok && k < want.length; k++) ok = names[nm + k] === want[k];
      if (ok) {
        if (off + size > data.length) throw new RangeError('section past the end');
        return data.slice(off, off + size);
      }
    }
  } catch (e) {
    if (e instanceof RangeError) throw new LogDataError(`truncated ELF file: ${e.message}`);
    throw e;
  }
  return undefined;
}

const INT_KINDS: Record<string, [number, boolean]> = {
  int32: [4, true],
  uint32: [4, false],
  pointer: [4, false],
  int64: [8, true],
  uint64: [8, false],
};

type Fmt = [string, string, string, string, CborValue[]] | CborValue[];

const utf8 = new TextDecoder('utf-8', { fatal: false });

function num(v: CborValue): number {
  return typeof v === 'bigint' ? Number(v) : (v as number);
}

/** One logdata file: the formats of one target (§6.1). */
export class LogData {
  readonly saddr: number;
  readonly fmts: Map<number, Fmt>;
  readonly enums: Map<string, Map<number, string>>;
  readonly tdenums: Map<string, Map<number, string>>;
  /** Sorted by address. */
  readonly fns: [number, number, string][];
  readonly vars: [number, string][];
  /** The app_hash from a `<hash>.logdata` file name, if any. */
  appHash?: Uint8Array;

  constructor(
    /** The `.logdata` CBOR (also for a file loaded from an ELF). */
    readonly cbor: Uint8Array,
    readonly source = '<bytes>',
  ) {
    const data = cbor;
    let a: CborValue;
    try {
      a = cborDecode(data);
    } catch (e) {
      throw new LogDataError(`${source}: not a logdata file (${(e as Error).message})`);
    }
    if (!(a instanceof Map)) throw new LogDataError(`${source}: not a logdata file (not a map)`);
    const need = (k: string) => {
      const v = a.get(k);
      if (v === undefined) throw new LogDataError(`${source}: not a logdata file (no ${k})`);
      return v;
    };
    try {
      this.saddr = num(need('saddr'));
      this.fmts = new Map();
      for (const [k, v] of need('fmts') as Map<CborValue, CborValue>) this.fmts.set(num(k), v as Fmt);
      const enumMap = (m: CborValue) => {
        const out = new Map<string, Map<number, string>>();
        for (const [n, vals] of m as Map<CborValue, CborValue>) {
          const e = new Map<number, string>();
          for (const [k, v] of vals as Map<CborValue, CborValue>) e.set(num(k), String(v));
          out.set(String(n), e);
        }
        return out;
      };
      this.enums = enumMap(need('enums'));
      this.tdenums = enumMap(need('tdenums'));
      this.fns = [];
      for (const [k, name] of need('fns') as Map<CborValue, CborValue>) {
        const [low, size] = k as CborValue[];
        this.fns.push([num(low), num(size), String(name)]);
      }
      this.fns.sort((x, y) => x[0] - y[0]);
      this.vars = [...(need('vars') as Map<CborValue, CborValue>)].map(([k, v]) => [num(k), String(v)] as [number, string]);
      this.vars.sort((x, y) => x[0] - y[0]);
    } catch (e) {
      if (e instanceof LogDataError) throw e;
      throw new LogDataError(`${source}: not a logdata file (${(e as Error).message})`);
    }
  }

  /** Load from file contents: `.logdata` CBOR, or an ELF with a `.logdata_cbor` section. */
  static fromFile(data: Uint8Array, name: string): LogData {
    if (data[0] === 0x7f && data[1] === 0x45 && data[2] === 0x4c && data[3] === 0x46) {
      const sec = elfSection(data, '.logdata_cbor');
      if (!sec || !sec.length) throw new LogDataError(`${name}: no .logdata_cbor section in ELF file`);
      data = sec;
    }
    const ld = new LogData(data, name);
    const stem = name.split(/[\\/]/).pop()!.split('.')[0];
    if (/^[0-9a-fA-F]{128}$/.test(stem)) {
      ld.appHash = new Uint8Array(stem.match(/../g)!.map((h) => parseInt(h, 16)));
    }
    return ld;
  }

  /** The target digit this logdata decodes. */
  get target(): number {
    return (this.saddr >>> TARGET_DIGIT_SHIFT) & 0xf;
  }

  lookupSym(a: number): string {
    // Clear the Thumb bit.
    const ca = (a & ~1) >>> 0;
    for (const [low, size, name] of this.fns) {
      if (low <= ca && ca < low + size) return `${name}+0x${(ca - low).toString(16)}`;
    }
    let best: [number, string] | undefined;
    for (const v of this.vars) {
      if (v[0] <= a) best = v;
      else break;
    }
    if (best && a - best[0] < 0x3000) return `${best[1]}+0x${(a - best[0]).toString(16)}`;
    return `0x${a.toString(16).padStart(8, '0')}`;
  }

  enumName(name: string, v: number): string {
    for (const t of [this.enums, this.tdenums]) {
      const e = t.get(name);
      if (e) return e.get(v) ?? `<${name}:${v}>`;
    }
    return `<!${name}:${v}>`;
  }

  /** Decode one type-0/1 log frame. Throws DecodeError. */
  decode(addr: number, args: Uint8Array): Decoded {
    const key = (addr & ~3) >>> 0;
    const entry = this.fmts.get(key);
    if (!entry || entry.length !== 5) {
      throw new DecodeError('unknown_format', `unknown format addr 0x${key.toString(16).padStart(8, '0')}`);
    }
    const [lvl, file, line, fmt, kinds] = entry as [CborValue, string, string, string, CborValue[]];
    const li = typeof lvl === 'string' && /^\d+$/.test(lvl) ? parseInt(lvl, 10) : typeof lvl === 'number' ? lvl : -1;
    const level = li >= 0 && li < LEVELS.length ? LEVELS[li] : `<bad level ${String(lvl)}>`;
    const where = `${file}:${line} "${fmt}"`;
    const missing = (i: number, kind: string) => new DecodeError('missing_arg', `${where} — missing arg ${i} (${kind})`);
    const dv = new DataView(args.buffer, args.byteOffset, args.byteLength);

    if ((addr & 3) === TYPE_MEM) {
      if (args.length < 4) throw missing(0, 'uint32');
      const ptr = dv.getUint32(0, true);
      const rest = args.subarray(4);
      if (line === '<zephyr>') return { level, file, line: '', text: utf8.decode(rest) };
      const bytes = Array.from(rest, (b) => b.toString(16).padStart(2, '0')).join(' ');
      return { level, file, line, text: `${fmt} ${ptr.toString(16).padStart(8, '0')}: ${bytes}` };
    }

    const values: PrintfArg[] = [];
    let o = 0;
    for (let i = 0; i < kinds.length; i++) {
      const k = kinds[i];
      const left = args.length - o;
      if (Array.isArray(k)) {
        if (left < 4) throw missing(i, 'enum');
        values.push(this.enumName(String(k[1]), dv.getInt32(o, true)));
        o += 4;
      } else if (typeof k === 'string' && k in INT_KINDS) {
        const [n, signed] = INT_KINDS[k];
        if (left < n) throw missing(i, k);
        const v =
          n === 4
            ? BigInt(signed ? dv.getInt32(o, true) : dv.getUint32(o, true))
            : signed
              ? dv.getBigInt64(o, true)
              : dv.getBigUint64(o, true);
        values.push(new IntArg(v, n));
        o += n;
      } else if (k === 'double') {
        if (left < 8) throw missing(i, k);
        values.push(dv.getFloat64(o, true));
        o += 8;
      } else if (k === 'sym') {
        if (left < 4) throw missing(i, k);
        values.push(this.lookupSym(dv.getUint32(o, true)));
        o += 4;
      } else if (k === 'string') {
        const z = args.indexOf(0, o);
        if (z < 0) throw missing(i, k);
        values.push(escapeStr(utf8.decode(args.subarray(o, z))));
        o = z + 1;
      } else if (k === 'bytes') {
        values.push(Array.from(args.subarray(o), (b) => b.toString(16).padStart(2, '0')).join(' '));
        o = args.length;
      } else {
        throw new DecodeError('format', `${where} — unknown argument kind ${JSON.stringify(k)}`);
      }
    }
    if (o < args.length) {
      const rest = args.subarray(o);
      throw new DecodeError('extra_bytes', `${where} — extra ${rest.length} bytes: ${hex(rest)}`);
    }
    try {
      return { level, file, line, text: formatC(fmt, values) };
    } catch (e) {
      throw new DecodeError('format', `${where} — ${(e as Error).message}`);
    }
  }
}

/** The logdata for each target digit. */
export class Decoders {
  private by = new Map<number, LogData>();
  insert(ld: LogData) {
    this.by.set(ld.target, ld);
  }
  get(target: number): LogData | undefined {
    return this.by.get(target);
  }
  clear() {
    this.by.clear();
  }
  get size() {
    return this.by.size;
  }
  targets(): number[] {
    return [...this.by.keys()].sort((a, b) => a - b);
  }
  all(): LogData[] {
    return [...this.by.values()];
  }
}
