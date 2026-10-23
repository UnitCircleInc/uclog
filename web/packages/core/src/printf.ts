// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * C printf, as the native tools format log lines (protocol.md §6.2). Corner cases need not match
 * any particular libc; the `logs.json` vectors must.
 */

/** An integer argument with its size in bytes (so %u/%x of a negative value wraps correctly). */
export class IntArg {
  constructor(
    readonly v: bigint,
    readonly size: number,
  ) {}
}

export type PrintfArg = IntArg | number | string;

const SPEC = /%([-+ #0]*)([0-9]*)(?:\.([0-9]*))?(?:hh|ll|h|l|z|j|t|L)?([diouxXcfFeEgGaAs%])/g;

function asBig(v: PrintfArg): bigint {
  if (v instanceof IntArg) return v.v;
  if (typeof v === 'number') return Number.isFinite(v) ? BigInt(Math.trunc(v)) : 0n;
  try {
    return BigInt(v.trim());
  } catch {
    return 0n;
  }
}

function asFloat(v: PrintfArg): number {
  if (v instanceof IntArg) return Number(v.v);
  if (typeof v === 'number') return v;
  const f = Number(v);
  return Number.isNaN(f) ? NaN : f;
}

function pad(flags: string, width: string, prefix: string, body: string, zeroOk: boolean): string {
  const w = parseInt(width || '0', 10);
  const n = w - prefix.length - body.length;
  if (n <= 0) return prefix + body;
  if (flags.includes('-')) return prefix + body + ' '.repeat(n);
  if (flags.includes('0') && zeroOk) return prefix + '0'.repeat(n) + body;
  return ' '.repeat(n) + prefix + body;
}

function signOf(neg: boolean, flags: string): string {
  if (neg) return '-';
  if (flags.includes('+')) return '+';
  if (flags.includes(' ')) return ' ';
  return '';
}

function floatHex(f: number): string {
  // Python float.hex(): 0x1.<13 hex digits>p<exp>
  if (f === 0) return (Object.is(f, -0) ? '-' : '') + '0x0.0p+0';
  const neg = f < 0;
  const dv = new DataView(new ArrayBuffer(8));
  dv.setFloat64(0, Math.abs(f));
  const bits = dv.getBigUint64(0);
  let exp = Number((bits >> 52n) & 0x7ffn);
  const mant = bits & 0xfffffffffffffn;
  let lead = '1';
  if (exp === 0) {
    lead = '0';
    exp = -1022;
  } else exp -= 1023;
  return `${neg ? '-' : ''}0x${lead}.${mant.toString(16).padStart(13, '0')}p${exp >= 0 ? '+' : ''}${exp}`;
}

function expForm(f: number, prec: number, upper: boolean): string {
  let s = f.toExponential(prec); // e.g. 1.234568e+4
  s = s.replace(/e([+-])(\d)$/, 'e$10$2');
  return upper ? s.toUpperCase() : s;
}

function gForm(f: number, precIn: number, alt: boolean, upper: boolean): string {
  const p = precIn === 0 ? 1 : precIn;
  if (f === 0) {
    const z = alt ? (0).toFixed(p - 1) : '0';
    return (Object.is(f, -0) ? '-' : '') + z;
  }
  const x = Math.floor(Math.log10(Math.abs(Number(f.toExponential(p - 1)))));
  let s: string;
  if (p > x && x >= -4) {
    s = f.toFixed(p - 1 - x);
    if (!alt && s.includes('.')) s = s.replace(/\.?0+$/, '');
  } else {
    s = f.toExponential(p - 1);
    let [m, e] = s.split('e');
    if (!alt && m.includes('.')) m = m.replace(/\.?0+$/, '');
    e = e.replace(/^([+-])(\d)$/, '$10$2');
    s = `${m}e${e}`;
  }
  return upper ? s.toUpperCase() : s;
}

function convert(flags: string, width: string, prec: string | undefined, conv: string, v: PrintfArg): string {
  const hasPrec = prec !== undefined;
  const p = parseInt(prec || '0', 10);
  if (conv === 'd' || conv === 'i') {
    const n = asBig(v);
    let digits = (n < 0n ? -n : n).toString();
    if (hasPrec) digits = p === 0 && n === 0n ? '' : digits.padStart(p, '0');
    return pad(flags, width, signOf(n < 0n, flags), digits, !hasPrec);
  }
  if ('ouxX'.includes(conv)) {
    let u = asBig(v);
    if (u < 0n) {
      const size = v instanceof IntArg ? v.size : 8;
      u &= (1n << BigInt(8 * size)) - 1n;
    }
    const alt = flags.includes('#') && u !== 0n;
    if (conv === 'u') {
      let digits = u.toString();
      if (hasPrec) digits = digits.padStart(p, '0');
      return pad(flags, width, '', digits, !hasPrec);
    }
    if (conv === 'o') {
      let s = u.toString(8);
      if (hasPrec) s = s.padStart(p, '0');
      if (alt && !s.startsWith('0')) s = '0' + s;
      return pad(flags, width, '', s, !hasPrec);
    }
    let digits = u.toString(16);
    if (conv === 'X') digits = digits.toUpperCase();
    if (hasPrec) digits = digits.padStart(p, '0');
    return pad(flags, width, alt ? '0' + conv : '', digits, !hasPrec);
  }
  if (conv === 'c') {
    const ch = typeof v === 'string' ? v.slice(0, 1) : String.fromCodePoint(Number(asBig(v) & 0x10ffffn));
    return pad(flags.replace('0', ''), width, '', ch, false);
  }
  if (conv === 's') {
    let t = typeof v === 'string' ? v : v instanceof IntArg ? v.v.toString() : String(v);
    if (hasPrec) t = [...t].slice(0, p).join('');
    return pad(flags.replace('0', ''), width, '', t, false);
  }
  const f = asFloat(v);
  const upper = conv === conv.toUpperCase();
  const neg = f < 0 || Object.is(f, -0);
  const a = Math.abs(f);
  let body: string;
  if (!Number.isFinite(f)) {
    body = Number.isNaN(f) ? 'nan' : 'inf';
    if (upper) body = body.toUpperCase();
    return pad(flags, width, signOf(neg && !Number.isNaN(f), flags), body, false);
  }
  const alt = flags.includes('#');
  const fp = hasPrec ? p : 6;
  switch (conv) {
    case 'f':
    case 'F':
      body = a.toFixed(fp);
      if (alt && !body.includes('.')) body += '.';
      break;
    case 'e':
    case 'E':
      body = expForm(a, fp, upper);
      if (alt && fp === 0) body = body.replace(/e/i, (m) => '.' + m);
      break;
    case 'g':
    case 'G':
      body = gForm(a, fp, alt, upper);
      break;
    default: {
      // a / A
      const h = floatHex(a);
      return pad(flags, width, signOf(neg, flags), upper ? h.toUpperCase().replace('0X', '0X') : h, false);
    }
  }
  return pad(flags, width, signOf(neg, flags), body, true);
}

/** Format with C printf semantics. Throws when conversions and values differ in number. */
export function formatC(fmt: string, values: PrintfArg[]): string {
  let out = '';
  let pos = 0;
  let k = 0;
  for (const m of fmt.matchAll(SPEC)) {
    out += fmt.slice(pos, m.index);
    pos = m.index! + m[0].length;
    const [, flags, width, prec, conv] = m;
    if (conv === '%') {
      out += '%';
      continue;
    }
    if (k >= values.length) throw new Error('format has more conversions than arguments');
    out += convert(flags, width, prec, conv, values[k++]);
  }
  out += fmt.slice(pos);
  if (k < values.length) throw new Error('format has fewer conversions than arguments');
  return out;
}

const ESC: Record<string, string> = { '\\': '\\\\', '\n': '\\n', '\r': '\\r', '\t': '\\t' };

/** Escape a string argument (§6.3). */
export function escapeStr(v: string): string {
  let out = '';
  for (const ch of v) {
    const e = ESC[ch];
    if (e) out += e;
    else {
      const c = ch.codePointAt(0)!;
      out += c < 0x20 || c === 0x7f ? `\\x${c.toString(16).padStart(2, '0')}` : ch;
    }
  }
  return out;
}
