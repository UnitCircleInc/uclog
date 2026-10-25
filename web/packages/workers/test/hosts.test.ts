// The two worker hosts wired by a real MessageChannel (no Worker threads), with the loopback
// device in the transport host.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { MemoryCache } from '@uclog/core';
import { TransportHost, MAX_OUTSTANDING } from '../src/transport-host.ts';
import type { DecodeToMain, TransportToMain } from '../src/protocol.ts';
import { DecodeHost as DH } from '../src/decode-host.ts';

const SAMPLE = new Uint8Array(readFileSync(join(__dirname, '../../../../spec/vectors/sample.logdata')));

async function until(what: string, f: () => boolean, ms = 4000) {
  const end = Date.now() + ms;
  while (!f()) {
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 10));
  }
}

function rig() {
  const fromT: TransportToMain[] = [];
  const fromD: DecodeToMain[] = [];
  const t = new TransportHost((m) => fromT.push(m));
  const d = new DH((m) => fromD.push(m), new MemoryCache());
  const ch = new MessageChannel();
  t.handle({ t: 'init', decoder: ch.port1 as unknown as MessagePort });
  ch.port2.onmessage = (e) => d.onTransport(e.data, (a) => ch.port2.postMessage(a));
  const close = () => {
    t.dispose();
    d.dispose();
    ch.port1.close();
    ch.port2.close();
  };
  return { t, d, fromT, fromD, close };
}

test('loopback through both hosts', async () => {
  const r = rig();
  try {
    r.d.handle({ t: 'load-logdata', id: 1, name: 'sample.logdata', data: SAMPLE });
    r.d.handle({ t: 'config', resolver: {}, maxLines: 1000, emitLogs: true });
    r.t.handle({ t: 'connect', link: { kind: 'loopback', opts: 'rate=100' }, fs: null });
    await until('logs', () => r.d.store.length > 20);
    expect(r.d.store.errors).toBe(0);
    expect(r.fromT.some((m) => m.t === 'link' && m.state === 'open')).toBe(true);
    await until('state', () => r.fromD.some((m) => m.t === 'state' && m.state.received > 5));
    await until('logs event', () => r.fromD.some((m) => m.t === 'logs'));
    const info = r.fromD.find((m) => m.t === 'device-info');
    expect(info && info.t === 'device-info' && info.fields.find(([k]) => k === 'board')?.[1]).toBe('loopback');
    // Terminal round trip.
    r.t.handle({ t: 'send', port: 61, data: new TextEncoder().encode('plot\r') });
    await until('plots', () => r.d.session.plots.plots.length === 3);
    await until('plot data', () => (r.d.plotData('loopback', null, null, 100) as { series: { x: Float64Array }[] }).series[0].x.length > 5);
    const pd = r.d.plotData('spectrum', null, null, 100)!;
    expect(pd.kind === 'grid' && pd.rows * pd.cols).toBe(3200);
    const win = r.d.store.window(0, 3);
    expect(win[0].text).toBe('Hello World! loopback start');
    r.t.handle({ t: 'disconnect' });
    expect(r.fromT.at(-1)).toMatchObject({ t: 'link', state: 'closed' });
  } finally {
    r.close();
  }
});

test('back-pressure: log frames dropped while the decoder does not acknowledge, then reported', async () => {
  const fromT: TransportToMain[] = [];
  const t = new TransportHost((m) => fromT.push(m));
  const ch = new MessageChannel();
  t.handle({ t: 'init', decoder: ch.port1 as unknown as MessagePort });
  const batches: { id: number; items: { t: string; kind?: string; text?: string }[] }[] = [];
  let ackOn = false;
  ch.port2.onmessage = (e) => {
    batches.push(e.data);
    if (ackOn) ch.port2.postMessage({ t: 'ack', id: e.data.id });
  };
  try {
    t.handle({ t: 'connect', link: { kind: 'loopback', opts: 'rate=0' }, fs: null });
    // Each burst produces at least one batch; with no acks the transport stops sending logs.
    for (let i = 0; i < MAX_OUTSTANDING + 10; i++) {
      t.handle({ t: 'send', port: 61, data: new TextEncoder().encode('burst 20\r') });
      await new Promise((r) => setTimeout(r, 25));
    }
    await until('batches', () => batches.length >= MAX_OUTSTANDING);
    const logsBefore = batches.flatMap((b) => b.items).filter((i) => i.t === 'log').length;
    expect(logsBefore).toBeLessThan((MAX_OUTSTANDING + 10) * 20);
    // Acknowledge everything; the next logs flow again with an error record first.
    ackOn = true;
    for (const b of batches) ch.port2.postMessage({ t: 'ack', id: b.id });
    await new Promise((r) => setTimeout(r, 50));
    t.handle({ t: 'send', port: 61, data: new TextEncoder().encode('burst 5\r') });
    await until('report', () => batches.flatMap((b) => b.items).some((i) => i.kind === 'decoder_behind'));
    const rec = batches.flatMap((b) => b.items).find((i) => i.kind === 'decoder_behind')!;
    expect(rec.text).toMatch(/dropped \d+ log frames/);
  } finally {
    t.dispose();
    ch.port1.close();
    ch.port2.close();
  }
});

test('one item that throws does not drop the rest of its batch', () => {
  const fromD: DecodeToMain[] = [];
  const d = new DH((m) => fromD.push(m), null);
  try {
    const orig = d.session.handle.bind(d.session);
    d.session.handle = (m) => {
      if (m.t === 'plot') throw new Error('boom');
      orig(m);
    };
    const acks: number[] = [];
    d.onTransport(
      {
        t: 'batch',
        id: 7,
        items: [
          { t: 'plot', data: new Uint8Array(0) },
          { t: 'error', ts: 1, kind: 'framing_invalid', text: 'after', target: null },
        ],
      },
      (a) => acks.push(a.id),
    );
    expect(acks).toEqual([7]);
    expect(d.store.window(0, 10).map((r) => [r.error, r.text])).toEqual([
      ['internal', 'decode: boom'],
      ['framing_invalid', 'after'],
    ]);
  } finally {
    d.dispose();
  }
});
