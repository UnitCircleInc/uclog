// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** FW update (protocol.md §5.4): message encoding for ports 59 and 3, and the client. */

import { decode, encode, get, getInt, getText, display, type CborValue } from './cbor.ts';
import { cobsDecode, cobsEncode, FrameScanner, PORTS } from './frame.ts';

let table: Uint32Array | undefined;

/** CRC-32C (Castagnoli). */
export function crc32c(data: Uint8Array): number {
  if (!table) {
    table = new Uint32Array(256);
    for (let i = 0; i < 256; i++) {
      let c = i;
      for (let k = 0; k < 8; k++) c = c & 1 ? (c >>> 1) ^ 0x82f63b78 : c >>> 1;
      table[i] = c >>> 0;
    }
  }
  let c = 0xffffffff;
  for (const b of data) c = table[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** The port-frame data for `msg` on port 59 (plain CBOR) or 3 (inner frame with CRC). */
export function fwEncode(port: number, msg: CborValue): Uint8Array {
  const body = encode(msg);
  if (port === PORTS.FW) return body;
  const withCrc = new Uint8Array(body.length + 4);
  withCrc.set(body);
  new DataView(withCrc.buffer).setUint32(body.length, crc32c(body), true);
  const e = cobsEncode(withCrc);
  const out = new Uint8Array(e.length + 2);
  out.set(e, 1);
  return out;
}

/** Decode port-frame data from port 59 or 3. Throws on a bad frame or CRC. */
export function fwDecode(port: number, data: Uint8Array): CborValue {
  if (port === PORTS.FW) return decode(data);
  const ev = new FrameScanner().feed(data);
  const f = ev.find((e) => 'frame' in e) as { frame: Uint8Array } | undefined;
  if (!f) throw new Error('no inner frame');
  const inner = cobsDecode(f.frame);
  if (inner.length < 4) throw new Error('short inner frame');
  const body = inner.subarray(0, inner.length - 4);
  const crc = new DataView(inner.buffer, inner.byteOffset).getUint32(inner.length - 4, true);
  if (crc !== crc32c(body)) throw new Error('CRC mismatch');
  return decode(body);
}

export type FwProgress =
  | { kind: 'probing' }
  | { kind: 'port'; port: number }
  | { kind: 'starting' }
  | { kind: 'block'; done: number; total: number }
  | { kind: 'scheduling' }
  | { kind: 'done'; port: number };

export class FwError extends Error {}

/** What the FW client needs from a session. */
export interface FwIo {
  send(port: number, data: Uint8Array): void;
  /** The next port-59/3 frame, or undefined after `ms` without one. Rejects when the session closes. */
  next(ms: number, signal?: AbortSignal): Promise<{ port: number; data: Uint8Array } | undefined>;
}

const now = () => Math.floor(Date.now() / 1000);

async function call(io: FwIo, port: number, cmd: string, msg: CborValue, ms: number, signal?: AbortSignal) {
  io.send(port, fwEncode(port, msg));
  const deadline = Date.now() + ms;
  for (;;) {
    if (signal?.aborted) throw new FwError('cancelled');
    const left = deadline - Date.now();
    if (left <= 0) return undefined;
    const r = await io.next(left, signal);
    if (!r) return undefined;
    if (r.port !== port) continue;
    let v: CborValue;
    try {
      v = fwDecode(port, r.data);
    } catch {
      continue;
    }
    if (get(v, 'event') !== undefined || getText(v, 'rsp') !== cmd) continue;
    const e = get(v, 'error');
    if (e !== undefined) throw new FwError(`device reported an error for ${cmd}: ${typeof e === 'string' ? e : display(e)}`);
    return v;
  }
}

async function must(p: Promise<CborValue | undefined>, cmd: string, ms: number) {
  const v = await p;
  if (v === undefined) throw new FwError(`no reply to ${cmd} within ${ms / 1000} s`);
  return v;
}

/** Run a FW update with the signed image; returns the port used (59 or 3). */
export async function fwUpdate(
  io: FwIo,
  image: Uint8Array,
  progress: (p: FwProgress) => void,
  signal?: AbortSignal,
): Promise<number> {
  if (!image.length) throw new FwError('image is empty');
  progress({ kind: 'probing' });
  let port: number = PORTS.FW;
  if ((await call(io, PORTS.FW, 'set-time', { cmd: 'set-time', timestamp: now() }, 1000, signal)) === undefined) {
    port = PORTS.FW_LEGACY;
    const r = await call(io, port, 'set-time', { cmd: 'set-time', timestamp: now() }, 5000, signal);
    if (r === undefined) throw new FwError('device does not support FW update (no reply on port 59 or port 3)');
  }
  progress({ kind: 'port', port });
  progress({ kind: 'starting' });
  const r = await must(call(io, port, 'fw-start', { cmd: 'fw-start' }, 60_000, signal), 'fw-start', 60_000);
  const block = getInt(r, 'block-size') ?? 0;
  if (block <= 0) throw new FwError('device reported an error for fw-start: reply has no block-size');
  const total = Math.ceil(image.length / block);
  for (let i = 0; i < total; i++) {
    const chunk = image.subarray(i * block, (i + 1) * block);
    await must(call(io, port, 'fw-block', { cmd: 'fw-block', data: chunk }, 5000, signal), 'fw-block', 5000);
    progress({ kind: 'block', done: i + 1, total });
  }
  progress({ kind: 'scheduling' });
  await must(call(io, port, 'fw-schedule', { cmd: 'fw-schedule', timestamp: now() + 5 }, 5000, signal), 'fw-schedule', 5000);
  progress({ kind: 'done', port });
  return port;
}
