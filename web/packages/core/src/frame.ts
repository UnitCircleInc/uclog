// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** Framing: COBS, the frame scanner, the 2-byte peek and the frame header (protocol.md §2–§3). */

/** §2.4: a frame on the wire (both delimiters) is at most 1452 bytes. */
export const MAX_WIRE = 1452;
/** The encoded run between the delimiters. */
export const MAX_FRAME = MAX_WIRE - 2;
export const MAX_PAYLOAD = 1444;
export const MAX_PORT_DATA = MAX_PAYLOAD - 1;

export const TYPE_LOG = 0;
export const TYPE_MEM = 1;
export const TYPE_RES = 2;
export const TYPE_PORT = 3;
export const TARGET_DIGIT_SHIFT = 20;

export const PORTS = {
  COMMANDS: 0,
  PRISM: 1,
  FW_LEGACY: 3,
  FW: 59,
  PLOT: 60,
  TERMINAL: 61,
  DEVICE_INFO: 62,
  SEMIHOST: 63,
} as const;

/** §2.3 timing, in milliseconds. */
export const PULSE_INTERVAL = 500;
export const LIVENESS_TIMEOUT = 2000;
export const WAKE_GAP = 50;

export const PULSE = new Uint8Array([0]);

export class CobsError extends Error {}

export function cobsEncode(data: Uint8Array): Uint8Array {
  const out = new Uint8Array(data.length + Math.ceil(data.length / 254) + 1);
  let o = 0;
  let codeAt = o++;
  let code = 1;
  for (let i = 0; i < data.length; i++) {
    const b = data[i];
    if (b === 0) {
      out[codeAt] = code;
      codeAt = o++;
      code = 1;
    } else {
      out[o++] = b;
      code++;
      if (code === 255) {
        out[codeAt] = code;
        // A full block that ends the input takes no further code byte (§2.1).
        if (i === data.length - 1) return out.slice(0, o);
        codeAt = o++;
        code = 1;
      }
    }
  }
  out[codeAt] = code;
  return out.slice(0, o);
}

export function cobsDecode(data: Uint8Array): Uint8Array {
  const n = data.length;
  if (n === 0) throw new CobsError('empty');
  const out = new Uint8Array(n);
  let o = 0;
  let i = 0;
  while (i < n) {
    const code = data[i];
    if (code === 0) throw new CobsError(`zero byte at ${i}`);
    const end = i + code;
    if (end > n) throw new CobsError(`code 0x${code.toString(16).padStart(2, '0')} at ${i} runs past the end (${n})`);
    for (let k = i + 1; k < end; k++) {
      if (data[k] === 0) throw new CobsError(`zero byte inside the block at ${i}`);
      out[o++] = data[k];
    }
    i = end;
    if (code < 255 || i === n) out[o++] = 0;
  }
  return out.slice(0, o - 1);
}

export type ScanEvent = { frame: Uint8Array } | { oversize: number };

/**
 * Splits a byte stream on 0x00 (§2.2). Bytes before the first 0x00 are dropped. Empty runs
 * (pulses, gaps) yield nothing; a run over `maxFrame` is discarded and reported.
 */
export class FrameScanner {
  private buf: Uint8Array;
  private n = 0;
  private discarding = 0;
  private seenDelim = false;

  constructor(readonly maxFrame = MAX_FRAME) {
    this.buf = new Uint8Array(maxFrame);
  }

  reset() {
    this.n = 0;
    this.discarding = 0;
    this.seenDelim = false;
  }

  private part(d: Uint8Array, a: number, b: number) {
    const k = b - a;
    if (this.discarding) {
      this.discarding += k;
    } else if (this.n + k > this.maxFrame) {
      this.discarding = this.n + k;
      this.n = 0;
    } else {
      this.buf.set(d.subarray(a, b), this.n);
      this.n += k;
    }
  }

  feed(data: Uint8Array, out: ScanEvent[] = []): ScanEvent[] {
    let i = 0;
    const n = data.length;
    if (!this.seenDelim) {
      const j = data.indexOf(0);
      if (j < 0) return out;
      this.seenDelim = true;
      i = j;
    }
    while (i < n) {
      const j = data.indexOf(0, i);
      if (j < 0) {
        this.part(data, i, n);
        break;
      }
      if (j > i) this.part(data, i, j);
      if (this.discarding) {
        out.push({ oversize: this.discarding });
        this.discarding = 0;
      } else if (this.n) {
        out.push({ frame: this.buf.slice(0, this.n) });
      }
      this.n = 0;
      i = j + 1;
    }
    return out;
  }
}

export type Peek = { kind: 'pulse' } | { kind: 'log'; type: number } | { kind: 'port'; port: number };

/** Classify an encoded (still COBS) frame from its first 2 bytes (§3.2). */
export function peek(e: Uint8Array): Peek {
  if (e.length === 0 || (e.length === 1 && e[0] === 1)) return { kind: 'pulse' };
  const b0 = e[0] === 1 ? 0 : e[1];
  const t = b0 & 3;
  return t === TYPE_PORT ? { kind: 'port', port: b0 >> 2 } : { kind: 'log', type: t };
}

export type Mux =
  | { kind: 'pulse' }
  | { kind: 'port'; port: number; data: Uint8Array }
  | { kind: 'log'; type: number; target: number; addr: number; args: Uint8Array }
  | { kind: 'short'; payload: Uint8Array };

export function muxDecode(p: Uint8Array): Mux {
  if (p.length === 0) return { kind: 'pulse' };
  const t = p[0] & 3;
  if (t === TYPE_PORT) return { kind: 'port', port: p[0] >> 2, data: p.subarray(1) };
  if (p.length < 4) return { kind: 'short', payload: p };
  const addr = (p[0] | (p[1] << 8) | (p[2] << 16) | (p[3] << 24)) >>> 0;
  return { kind: 'log', type: t, target: (addr >>> TARGET_DIGIT_SHIFT) & 0xf, addr, args: p.subarray(4) };
}

export function muxEncode(port: number, data: Uint8Array): Uint8Array {
  if (!(port >= 0 && port <= 63)) throw new RangeError(`port ${port} out of range 0..63`);
  const out = new Uint8Array(data.length + 1);
  out[0] = (port << 2) | TYPE_PORT;
  out.set(data, 1);
  return out;
}

/** The complete on-the-wire frame, both delimiters included (§2.1). */
export function wire(payload: Uint8Array): Uint8Array {
  const e = cobsEncode(payload);
  const out = new Uint8Array(e.length + 2);
  out.set(e, 1);
  return out;
}

export function wirePort(port: number, data: Uint8Array): Uint8Array {
  return wire(muxEncode(port, data));
}

// ---------------------------------------------------------------------------------------------
// Link specifics (§4)
// ---------------------------------------------------------------------------------------------

export const HID_IN = 0xa1;
export const HID_OUT = 0xa2;
export const HID_PAYLOAD = 63;

/** §4.3: a byte stream as 63-byte output reports, the last one zero padded (report data only). */
export function hidReports(data: Uint8Array): Uint8Array[] {
  const out: Uint8Array[] = [];
  for (let i = 0; i < data.length; i += HID_PAYLOAD) {
    const r = new Uint8Array(HID_PAYLOAD);
    r.set(data.subarray(i, i + HID_PAYLOAD));
    out.push(r);
  }
  return out;
}

/** §4.1 STLINK workaround: a write of `len % 8 == 0` goes out as `len-1` bytes, then 1 byte. */
export function stlinkSplit(data: Uint8Array): Uint8Array[] {
  if (data.length > 1 && data.length % 8 === 0) return [data.subarray(0, data.length - 1), data.subarray(data.length - 1)];
  return [data];
}

/** §4.1 default serial filters (D1). */
export const SERIAL_FILTERS: { usbVendorId: number; usbProductId?: number }[] = [
  ...[0x374b, 0x374e, 0x374f, 0x3752, 0x3753, 0x3754].map((p) => ({ usbVendorId: 0x0483, usbProductId: p })),
  ...[0x6001, 0x6010, 0x6011, 0x6014, 0x6015].map((p) => ({ usbVendorId: 0x0403, usbProductId: p })),
  { usbVendorId: 0x2fe3 },
];
export const HID_VENDOR = 0x2fe3;
export const BLE_SERVICE = '8683b8f3-0000-1000-8000-00805f9b34fb';
