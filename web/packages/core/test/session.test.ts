// Transport + decode halves against the simulated loopback device, plus the FW update, the
// semihost sandbox, the resolver and the stores.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import {
  decode,
  DecodeSession,
  encode,
  FrameScanner,
  cobsDecode,
  muxDecode,
  formatC,
  fwEncode,
  hex,
  IntArg,
  LogStore,
  Filter,
  LoopbackDevice,
  LOOPBACK_APP_HASH,
  MemoryCache,
  MemoryFs,
  OP,
  PORTS,
  Resolver,
  SemihostHandler,
  TransportSession,
  wirePort,
  type DecoderEvent,
  type ToUi,
  type CborValue,
  getText,
} from '../src/index.ts';

const SAMPLE = new Uint8Array(readFileSync(join(__dirname, '../../../../spec/vectors/sample.logdata')));
const te = new TextEncoder();
const td = new TextDecoder();

async function until(what: string, f: () => boolean, ms = 3000) {
  const end = Date.now() + ms;
  while (!f()) {
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 5));
  }
}

function rig(opts: { rate?: number; manual?: boolean; cache?: MemoryCache; fs?: MemoryFs } = {}) {
  const store = new LogStore(1000);
  const events: DecoderEvent[] = [];
  const ui: ToUi[] = [];
  const resolver = new Resolver({}, opts.cache ?? null, null);
  const dec = new DecodeSession(resolver, (r) => store.push(r), (e) => events.push(e));
  if (opts.manual !== false) dec.loadManual(SAMPLE, 'sample.logdata');
  let dev: LoopbackDevice;
  const t = new TransportSession(
    { hub: false, write: (b) => dev.write(b) },
    (m) => dec.handle(m),
    (m) => ui.push(m),
    { fs: opts.fs ?? null },
  );
  dev = new LoopbackDevice({ rate: opts.rate ?? 0, board: 'sim' }, (b) => queueMicrotask(() => t.onData(b)));
  const tick = setInterval(() => dec.tick(), 50);
  t.open();
  dev.start();
  const term = () => ui.filter((m) => m.t === 'port' && m.port === 61).map((m) => td.decode((m as { data: Uint8Array }).data)).join('');
  const close = () => {
    clearInterval(tick);
    t.close();
    dev.close();
  };
  return { store, events, ui, dec, t, dev, term, close };
}

describe('session with the loopback device', () => {
  test('device info selects the manual logdata; logs decode; terminal echo', async () => {
    const r = rig({ rate: 50 });
    try {
      await until('logs', () => r.store.length >= 5);
      const first = r.store.visibleAt(0)!;
      expect(first.text).toBe('Hello World! loopback start');
      expect(r.store.errors).toBe(0);
      const info = r.events.find((e) => e.t === 'device-info');
      expect(info && info.t === 'device-info' && info.appHash).toBe(hex(LOOPBACK_APP_HASH));
      expect(r.events.some((e) => e.t === 'logdata' && e.source?.kind === 'manual')).toBe(true);
      r.t.send(PORTS.TERMINAL, te.encode('echo hello web\r'));
      await until('echo', () => r.term().includes('hello web\r\n'));
      expect(r.ui.some((m) => m.t === 'liveness' && m.state === 'alive')).toBe(true);
    } finally {
      r.close();
    }
  });

  test('no logdata: error records with the raw bytes', async () => {
    const r = rig({ rate: 50, manual: false });
    try {
      await until('records', () => r.store.length >= 3);
      expect(r.store.visibleAt(0)!.error).toBe('no_logdata');
      expect(r.store.visibleAt(0)!.text).toMatch(/^decode: no logdata for target 1 \(addr 0xf0100000\) \[/);
      expect(r.events.some((e) => e.t === 'logdata' && e.source === null)).toBe(true);
    } finally {
      r.close();
    }
  });

  test('a manual file is remembered in the cache under the app_hash', async () => {
    const cache = new MemoryCache();
    const r = rig({ cache });
    try {
      await until('cached', () => cache.map.has(hex(LOOPBACK_APP_HASH)));
    } finally {
      r.close();
    }
    const r2 = rig({ cache, manual: false, rate: 50 });
    try {
      await until('logs', () => r2.store.length >= 2);
      expect(r2.events.some((e) => e.t === 'logdata' && e.source?.kind === 'cache')).toBe(true);
      expect(r2.store.errors).toBe(0);
    } finally {
      r2.close();
    }
  });

  test('semihost: time op, stdout write, stdin read', async () => {
    const r = rig();
    try {
      r.t.send(PORTS.TERMINAL, te.encode('sh\r'));
      await until('sh done', () => r.term().includes('sh: WRITE r=Some(20) errno=Some(0)'));
      expect(r.term()).toMatch(/sh: TIMEOFDAY sec=Some\(\d+\) \(12 bytes\)/);
      const out = r.ui.find((m) => m.t === 'console');
      expect(out && td.decode((out as { data: Uint8Array }).data)).toBe('hello from loopback\n');
      r.t.send(PORTS.TERMINAL, te.encode('read\r'));
      await until('stdin wanted', () => r.ui.some((m) => m.t === 'stdin-wanted'));
      r.t.stdin(te.encode('typed\r'));
      await until('read reply', () => r.term().includes('read: n=6 errno=Some(0) "typed\\n"'));
    } finally {
      r.close();
    }
  });

  test('plots', async () => {
    const r = rig();
    try {
      r.t.send(PORTS.TERMINAL, te.encode('plot\r'));
      await until('plot data', () => r.dec.plots.plots.length === 3 && r.dec.plots.frames > 20);
      const l = r.dec.plots.get('loopback')!.p;
      expect(l.kind === 'line' && l.axes.length).toBe(2);
      expect(l.kind === 'line' && l.axes[0].series[0].y.length).toBeGreaterThan(10);
      expect(r.dec.plots.csv('loopback')!.split('\n')[0]).toBe('t (s),sin,cos,ramp');
      r.t.send(PORTS.TERMINAL, te.encode('plot stop\r'));
    } finally {
      r.close();
    }
  });

  test('oversize send is refused with an error record', () => {
    const r = rig({ manual: false });
    try {
      r.t.send(0, new Uint8Array(1444));
      expect(r.store.visibleAt(r.store.visibleLength - 1)!.error).toBe('send_oversize');
    } finally {
      r.close();
    }
  });
});

/** A device that implements the FW update on `ports` (59 and/or 3). */
function fwDevice(ports: number[], blockSize = 100) {
  const got: { cmd: string; n?: number }[] = [];
  const sc = new FrameScanner();
  let t!: TransportSession;
  const write = (b: Uint8Array) => {
    for (const e of sc.feed(b)) {
      if (!('frame' in e)) continue;
      const m = muxDecode(cobsDecode(e.frame));
      if (m.kind !== 'port' || !ports.includes(m.port)) continue;
      const port = m.port;
      let msg: CborValue;
      if (port === 59) msg = decode(m.data);
      else {
        const inner = cobsDecode((new FrameScanner().feed(m.data)[0] as { frame: Uint8Array }).frame);
        msg = decode(inner.subarray(0, inner.length - 4));
      }
      const cmd = getText(msg, 'cmd')!;
      const data = (msg as Map<CborValue, CborValue>).get('data') as Uint8Array | undefined;
      got.push({ cmd, n: data?.length });
      const rsp: Record<string, CborValue> = { rsp: cmd };
      if (cmd === 'fw-start') rsp['block-size'] = blockSize;
      queueMicrotask(() => {
        t.onData(wirePort(port, fwEncode(port, { event: 'progress' })));
        t.onData(wirePort(port, fwEncode(port, rsp)));
      });
    }
  };
  t = new TransportSession({ hub: true, write }, () => {}, () => {});
  return { t, got };
}

describe('FW update', () => {
  test('port 59', async () => {
    const { t, got } = fwDevice([59]);
    const prog: string[] = [];
    const port = await t.fwUpdate(new Uint8Array(250), (p) => prog.push(p.kind));
    expect(port).toBe(59);
    expect(got.map((g) => g.cmd)).toEqual(['set-time', 'fw-start', 'fw-block', 'fw-block', 'fw-block', 'fw-schedule']);
    expect(got.filter((g) => g.n).map((g) => g.n)).toEqual([100, 100, 50]);
    expect(prog).toEqual(['probing', 'port', 'starting', 'block', 'block', 'block', 'scheduling', 'done']);
    t.close();
  });

  test('falls back to port 3', async () => {
    const { t, got } = fwDevice([3]);
    expect(await t.fwUpdate(new Uint8Array(10), () => {})).toBe(3);
    expect(got.map((g) => g.cmd)).toEqual(['set-time', 'fw-start', 'fw-block', 'fw-schedule']);
    t.close();
  }, 10_000);

  test('cancel', async () => {
    const { t } = fwDevice([]);
    const ac = new AbortController();
    setTimeout(() => ac.abort(), 50);
    await expect(t.fwUpdate(new Uint8Array(10), () => {}, ac.signal)).rejects.toThrow(/cancelled/);
    t.close();
  });
});

describe('semihost sandbox', () => {
  const req = (op: number, ...parts: (number | string | Uint8Array)[]) => {
    const bs = parts.map((p) => {
      if (typeof p === 'string') return te.encode(p);
      if (p instanceof Uint8Array) return p;
      const b = new Uint8Array(4);
      new DataView(b.buffer).setInt32(0, p, true);
      return b;
    });
    const out = new Uint8Array(1 + bs.reduce((n, b) => n + b.length, 0));
    out[0] = op;
    let o = 1;
    for (const b of bs) {
      out.set(b, o);
      o += b.length;
    }
    return out;
  };
  const rv = (o: { reply?: Uint8Array } | { stdin: number }) => {
    const r = (o as { reply: Uint8Array }).reply;
    const dv = new DataView(r.buffer);
    return [dv.getInt32(0, true), dv.getInt32(4, true), r.subarray(8)] as const;
  };

  test('open, write, read back, fstat, close', async () => {
    const fs = new MemoryFs();
    const h = new SemihostHandler(fs);
    expect(rv(await h.handle(req(OP.MKDIR, 0o755, 'out'))).slice(0, 2)).toEqual([0, 0]);
    const [fid] = rv(await h.handle(req(OP.OPEN, 0x601, 'out/log.txt')));
    expect(fid).toBe(3);
    expect(rv(await h.handle(req(OP.WRITE, fid, 'abc'))).slice(0, 2)).toEqual([3, 0]);
    const st = (await h.handle(req(OP.FSTAT, fid))) as { reply: Uint8Array };
    expect(st.reply.length).toBe(48);
    expect(new DataView(st.reply.buffer).getUint32(8 + 6 * 4, true)).toBe(3);
    expect(rv(await h.handle(req(OP.CLOSE, fid)))[0]).toBe(0);
    const [rfid] = rv(await h.handle(req(OP.OPEN, 0, 'out/log.txt')));
    const [n, , data] = rv(await h.handle(req(OP.READ, rfid, 64)));
    expect([n, td.decode(data)]).toEqual([3, 'abc']);
    expect(rv(await h.handle(req(OP.CLOSE, 99)))[1]).toBe(9); // EBADF
  });

  test('escapes and missing sandbox', async () => {
    const h = new SemihostHandler(new MemoryFs());
    for (const p of ['/etc/passwd', '../x', 'a/../../x', 'C:\\x']) {
      expect(rv(await h.handle(req(OP.OPEN, 0, p))).slice(0, 2), p).toEqual([-1, 13]);
    }
    expect(rv(await h.handle(req(OP.OPEN, 0, 'missing'))).slice(0, 2)).toEqual([-1, 2]);
    const none = new SemihostHandler(null);
    expect(rv(await none.handle(req(OP.OPEN, 0, 'x'))).slice(0, 2)).toEqual([-1, 13]);
    expect(rv(await none.handle(req(0x20, 1))).slice(0, 2)).toEqual([-1, 88]);
    expect(rv(await none.handle(req(OP.CLOSE, 1, 2))).slice(0, 2)).toEqual([-1, 22]); // malformed
  });
});

describe('resolver', () => {
  const hash = new Uint8Array(64).fill(7);
  test('remote fetch with token, cached afterwards', async () => {
    const calls: [string, string | null][] = [];
    const f = (async (url: string, init?: RequestInit) => {
      calls.push([url, new Headers(init?.headers).get('authorization')]);
      return new Response(SAMPLE);
    }) as typeof fetch;
    const cache = new MemoryCache();
    const r = new Resolver({ domain: 'acme.example', token: 'tok', url: 'https://ld.example/' }, cache, f);
    const out = await r.resolve(hash);
    expect(out.source?.kind).toBe('remote');
    expect(calls).toEqual([[`https://ld.example/logdata/acme.example/${hex(hash)}`, 'Bearer tok']]);
    expect(cache.map.has(hex(hash))).toBe(true);
    expect((await r.resolve(hash)).source?.kind).toBe('cache');
  });

  test('HTTP errors carry the problem detail and request id', async () => {
    const f = (async () =>
      new Response(JSON.stringify({ title: 'Forbidden', detail: 'token has no access' }), {
        status: 403,
        headers: { 'x-request-id': 'r-1', 'content-type': 'application/problem+json' },
      })) as unknown as typeof fetch;
    const out = await new Resolver({ domain: 'd' }, null, f).resolve(hash);
    expect(out.source).toBeNull();
    expect(out.notes[0]).toMatch(/HTTP 403: token has no access \(request id r-1\)/);
  });

  test('404 → not found', async () => {
    const f = (async () => new Response('', { status: 404 })) as unknown as typeof fetch;
    const out = await new Resolver({ domain: 'd' }, null, f).resolve(hash);
    expect(out.notes[0]).toMatch(/not found/);
  });
});

describe('printf', () => {
  const i = (v: number, size = 4) => new IntArg(BigInt(v), size);
  test.each([
    ['%5.2f|%-8.3e|%g|%g|%g', [3.14159, 12345.678, 0.0001, 123456789, 100], ' 3.14|1.235e+04|0.0001|1.23457e+08|100'],
    ['%#x %#o %X %u', [i(255), i(8), i(0xabc), i(-1)], '0xff 010 ABC 4294967295'],
    ['%+d % d %05d %.3d', [i(5), i(5), i(-42), i(7)], '+5  5 -0042 007'],
    ['%c%c %s|%.2s|%5s', [i(65), i(0x263a), 'x', 'abc', 'ab'], 'A☺ x|ab|   ab'],
    ['%a %f %e', [1.0, Infinity, NaN], '0x1.0000000000000p+0 inf nan'],
    ['%lld %llu', [new IntArg(-1n, 8), new IntArg(-1n, 8)], '-1 18446744073709551615'],
  ] as const)('%s', (fmt, args, want) => {
    expect(formatC(fmt, [...args])).toBe(want);
  });
  test('argument count mismatch throws', () => {
    expect(() => formatC('%d %d', [i(1)])).toThrow();
    expect(() => formatC('%d', [i(1), i(2)])).toThrow();
  });
});

describe('log store', () => {
  const rec = (level: string, file: string, text: string, error?: string) => ({ ts: 0, target: 1, level, file, line: '1', text, error });
  test('filters, error records always shown, ring bound', () => {
    const s = new LogStore(100);
    for (let k = 0; k < 150; k++) s.push(rec(k % 2 ? 'INFO' : 'WARN', k % 3 ? 'a.c' : 'b.c', `m${k}`));
    expect(s.length).toBe(100);
    expect(s.visibleAt(0)!.text).toBe('m50');
    s.push(rec('ERROR', '<uclog>', 'decode: x', 'no_logdata'));
    s.setFilter(new Filter({ minLevel: 2, source: 'b.c', line: '', search: '' }));
    expect(s.window(0, 1000).every((r) => r.error || (r.level === 'WARN' && r.file === 'b.c'))).toBe(true);
    expect(s.window(0, 1000).at(-1)!.error).toBe('no_logdata');
    s.setFilter(new Filter({ minLevel: 0, source: '', line: '', search: 'm1[0-4]0' }));
    expect(s.window(0, 10).map((r) => r.text)).toEqual(['m100', 'm110', 'm120', 'm130', 'm140']);
    expect(() => new Filter({ minLevel: 0, source: '', line: '', search: '(' })).toThrow();
    expect(s.errors).toBe(1);
    expect(s.csv().split('\n')[1]).toBe('0.000,WARN,"a.c","1","m100"');
  });
});

test('cbor round trip and map keys', () => {
  const v = encode({ a: 1, b: [1.5, -2, 'x', new Uint8Array([1, 2])], c: null, d: true, big: 2n ** 60n });
  const d = decode(v) as Map<CborValue, CborValue>;
  expect(d.get('a')).toBe(1);
  expect(d.get('b')).toEqual([1.5, -2, 'x', new Uint8Array([1, 2])]);
  expect(d.get('big')).toBe(2n ** 60n);
});
