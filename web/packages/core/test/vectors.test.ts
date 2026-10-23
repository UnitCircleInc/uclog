// The shared protocol vectors (spec/vectors), read the way spec/reference/check_vectors.py reads them.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import {
  cobsDecode,
  cobsEncode,
  crc32c,
  decode,
  DecodeError,
  ERRNO,
  errorReply,
  FrameScanner,
  fromHex,
  fwDecode,
  hex,
  hidReports,
  HID_IN,
  isTimeOp,
  LogData,
  muxDecode,
  parseRequest,
  peek,
  stlinkSplit,
  timeReply,
  type CborValue,
} from '../src/index.ts';

const VEC = join(__dirname, '../../../../spec/vectors');
const load = (n: string) => JSON.parse(readFileSync(join(VEC, n), 'utf8'));
const B = fromHex;

function scanOne(w: Uint8Array) {
  const ev = new FrameScanner().feed(w);
  expect(ev.length).toBe(1);
  return cobsDecode((ev[0] as { frame: Uint8Array }).frame);
}

/** Decoded CBOR → plain values comparable with the vectors' JSON (bytes as {hex}). */
function plain(v: CborValue): unknown {
  if (v instanceof Map) return Object.fromEntries([...v].map(([k, x]) => [String(k), plain(x)]));
  if (v instanceof Uint8Array) return { hex: hex(v) };
  if (Array.isArray(v)) return v.map(plain);
  return v;
}

describe('cobs.json', () => {
  const v = load('cobs.json');
  for (const c of v.valid) {
    test(`valid: ${c.name}`, () => {
      expect(hex(cobsEncode(B(c.decoded)))).toBe(c.encoded);
      expect(hex(cobsDecode(B(c.encoded)))).toBe(c.decoded);
    });
  }
  for (const c of v.invalid) {
    test(`invalid: ${c.name}`, () => expect(() => cobsDecode(B(c.encoded))).toThrow());
  }
});

test('scanner.json', () => {
  for (const c of load('scanner.json').cases) {
    const sc = new FrameScanner(c.max_frame);
    const ev = [];
    for (const ch of c.chunks) ev.push(...sc.feed(B(ch)));
    const got = ev.map((e) => ('frame' in e ? { frame: hex(e.frame) } : { oversize: e.oversize }));
    expect(got, c.name).toEqual(c.events);
  }
});

test('peek_mux.json', () => {
  for (const c of load('peek_mux.json').cases) {
    const enc = B(c.encoded);
    const pk = peek(enc.length > 2 && !(enc.length === 1 && enc[0] === 1) ? enc.subarray(0, 2) : enc);
    expect(pk.kind, c.name).toBe(c.peek.kind);
    if (pk.kind === 'port') expect(pk.port).toBe(c.peek.port);
    if (pk.kind === 'log') expect(pk.type).toBe(c.peek.type);
    const m = muxDecode(cobsDecode(enc));
    expect(m.kind === 'short' ? 'error' : m.kind, c.name).toBe(c.mux.kind);
    if (m.kind === 'port') expect([m.port, hex(m.data)]).toEqual([c.mux.port, c.mux.data]);
    if (m.kind === 'log') expect([m.type, m.target, m.addr, hex(m.args)]).toEqual([c.mux.type, c.mux.target, c.mux.addr, c.mux.args]);
  }
});

describe('logs.json', () => {
  const v = load('logs.json');
  const ld = new LogData(new Uint8Array(readFileSync(join(VEC, v.logdata))));
  test('target', () => expect(ld.target).toBe(v.logdata_target));
  for (const c of v.cases) {
    test(c.name, () => {
      const m = muxDecode(scanOne(B(c.wire)));
      if (m.kind !== 'log') throw new Error('not a log');
      expect(m.target).toBe(c.target);
      expect(ld.decode(m.addr, m.args)).toEqual(c.expect);
    });
  }
  for (const c of v.errors) {
    test(`error: ${c.name}`, () => {
      const m = muxDecode(scanOne(B(c.wire)));
      if (m.kind !== 'log') throw new Error('not a log');
      try {
        ld.decode(m.addr, m.args);
        throw new Error('decoded');
      } catch (e) {
        expect(e).toBeInstanceOf(DecodeError);
        expect((e as DecodeError).kind).toBe(c.error);
      }
    });
  }
  test('no logdata for target', () => {
    const c = v.no_logdata_for_target;
    const m = muxDecode(scanOne(B(c.wire)));
    expect(m.kind === 'log' && m.target).toBe(c.target);
    expect(c.target).not.toBe(ld.target);
  });
});

test('fw_update.json', () => {
  const v = load('fw_update.json');
  expect(crc32c(B(v.crc32c_check.input))).toBe(v.crc32c_check.crc);
  for (const c of [...v.requests, ...v.replies]) {
    for (const [port, key] of [
      [59, 'port59_wire'],
      [3, 'port3_wire'],
    ] as const) {
      const m = muxDecode(scanOne(B(c[key])));
      if (m.kind !== 'port') throw new Error('not a port frame');
      expect(m.port).toBe(port);
      expect(plain(fwDecode(port, m.data))).toEqual(c.msg);
    }
  }
});

test('semihost.json', () => {
  const v = load('semihost.json');
  for (const c of v.requests) {
    const m = muxDecode(scanOne(B(c.wire)));
    if (m.kind !== 'port') throw new Error('not a port frame');
    expect([m.port, hex(m.data)]).toEqual([63, c.request]);
    const r = parseRequest(B(c.request));
    const fields = Object.fromEntries(Object.entries(r.fields).map(([k, x]) => [k, x instanceof Uint8Array ? hex(x) : x]));
    expect([r.op, r.name, fields], c.name).toEqual([c.op, c.op_name, c.fields]);
    expect(isTimeOp(r.op)).toBe(c.time_op);
  }
  for (const c of v.time_replies) {
    const i = c.inputs;
    const [ticks, freq] = i.qpc !== undefined ? [BigInt(i.qpc), BigInt(i.qpf)] : [BigInt(i.mono_ns), 1_000_000_000n];
    expect(hex(timeReply(c.op, ticks, freq, i.wall)!), `${c.source} ${c.op_name}`).toBe(c.reply);
  }
  for (const c of v.errors) {
    expect(ERRNO[c.errno as keyof typeof ERRNO]).toBe(c.value);
    expect(hex(errorReply(c.op, c.errno)), c.name).toBe(c.reply);
  }
  for (const [k, n] of Object.entries(v.newlib_errno)) expect(ERRNO[k as keyof typeof ERRNO], k).toBe(n);
});

test('links.json', () => {
  const v = load('links.json');
  for (const c of v.hid_out) {
    expect(hidReports(B(c.data)).map((r) => 'a2' + hex(r)), c.name).toEqual(c.reports);
  }
  for (const c of v.stlink) expect(stlinkSplit(B(c.data)).map((w) => hex(w)), c.name).toEqual(c.writes);
  const e = v.hid_in_example;
  expect(B(e.report)[0]).toBe(HID_IN);
  expect(hex(B(e.report).subarray(1))).toBe(e.scanner_bytes);
});

test('device_info.json', () => {
  const v = load('device_info.json');
  for (const c of v.cases) {
    const m = muxDecode(scanOne(B(c.wire)));
    if (m.kind !== 'port') throw new Error('not a port frame');
    const info = decode(m.data) as Map<CborValue, CborValue>;
    expect(m.port).toBe(62);
    expect(hex(info.get('app_hash') as Uint8Array)).toBe(c.app_hash_hex);
  }
  const m = muxDecode(scanOne(B(v.request.wire)));
  expect(m.kind === 'port' && [m.port, m.data.length]).toEqual([62, 0]);
});
