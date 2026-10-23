// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * The transport half of a session (plan 02 §3): framing, pulses and the wake gap, liveness, port
 * routing, semihost replies and the FW update. It never decodes logs: log frames, port-60/62
 * payloads and error records go to the decode half through `toDecoder`.
 */

import { hex } from './cbor.ts';
import {
  cobsDecode,
  FrameScanner,
  LIVENESS_TIMEOUT,
  MAX_FRAME,
  MAX_PORT_DATA,
  muxDecode,
  PORTS,
  PULSE,
  PULSE_INTERVAL,
  TYPE_RES,
  WAKE_GAP,
  wirePort,
} from './frame.ts';
import { fwUpdate, type FwIo, type FwProgress } from './fw.ts';
import { isTimeOp, SemihostHandler, type FsProvider } from './semihost.ts';

/** Bytes to and from the device or hub. */
export interface RawLink {
  write(b: Uint8Array): Promise<void> | void;
  /** A hub link: the hub asks for the device info and answers time ops itself. */
  readonly hub: boolean;
}

export type Liveness = 'waiting' | 'alive' | 'not-responding';

export type ToDecoder =
  | { t: 'start'; direct: boolean }
  | { t: 'log'; ts: number; target: number; addr: number; args: Uint8Array }
  | { t: 'error'; ts: number; kind: string; text: string; target: number | null }
  | { t: 'info'; ts: number; data: Uint8Array }
  | { t: 'plot'; data: Uint8Array };

export type ToUi =
  | { t: 'liveness'; state: Liveness }
  | { t: 'port'; port: number; data: Uint8Array }
  | { t: 'console'; fid: number; data: Uint8Array }
  | { t: 'stdin-wanted' }
  | { t: 'note'; text: string };

export interface TransportOptions {
  maxFrame?: number;
  fs?: FsProvider | null;
  semihost?: SemihostHandler;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

export class TransportSession {
  readonly start = performance.now();
  private scanner: FrameScanner;
  readonly semihost: SemihostHandler;
  private liveness: Liveness = 'waiting';
  private lastRx: number | null = null;
  private lastTx: number | null = null;
  private ready = 0;
  private queue: Uint8Array[] = [];
  private pumping = false;
  private closed = false;
  private timer: ReturnType<typeof setInterval> | undefined;
  private unknownPorts = new Map<number, number>();
  private stdinWanted: number | null = null;
  private shChain: Promise<void> = Promise.resolve();
  private fwQueue: { port: number; data: Uint8Array }[] | null = null;
  private fwWake: (() => void) | null = null;
  /** Bytes received and sent, for the status bar. */
  rxBytes = 0;
  txBytes = 0;

  constructor(
    private link: RawLink,
    private toDecoder: (m: ToDecoder) => void,
    private toUi: (m: ToUi) => void,
    opts: TransportOptions = {},
  ) {
    this.scanner = new FrameScanner(opts.maxFrame ?? MAX_FRAME);
    this.semihost = opts.semihost ?? new SemihostHandler(opts.fs ?? null);
  }

  /** Begin: start the pulse timer and, on a direct link, ask for the device info (F7). */
  open() {
    this.toUi({ t: 'liveness', state: 'waiting' });
    this.toDecoder({ t: 'start', direct: !this.link.hub });
    if (!this.link.hub) this.enqueue(wirePort(PORTS.DEVICE_INFO, new Uint8Array(0)));
    this.timer = setInterval(() => this.tick(), PULSE_INTERVAL / 2);
  }

  close() {
    this.closed = true;
    clearInterval(this.timer);
    this.fwWake?.();
    void this.semihost.closeAll();
  }

  private now() {
    return performance.now();
  }

  private ts() {
    return (this.now() - this.start) / 1000;
  }

  private idle() {
    return this.lastTx === null || this.now() - this.lastTx > LIVENESS_TIMEOUT;
  }

  private async write(b: Uint8Array) {
    this.txBytes += b.length;
    await this.link.write(b);
    this.lastTx = this.now();
  }

  private tick() {
    if (this.closed) return;
    if (!this.pumping && (this.lastTx === null || this.now() - this.lastTx >= PULSE_INTERVAL)) {
      if (this.idle()) this.ready = this.now() + WAKE_GAP;
      this.write(PULSE).catch(() => {});
    }
    if (this.liveness === 'alive' && this.lastRx !== null && this.now() - this.lastRx > LIVENESS_TIMEOUT) {
      this.liveness = 'not-responding';
      this.toUi({ t: 'liveness', state: 'not-responding' });
    }
  }

  /** Queue a complete wire frame; after more than 2 s of TX idle a wake pulse and gap go first (§2.3). */
  private enqueue(frame: Uint8Array) {
    this.queue.push(frame);
    if (!this.pumping) void this.pump();
  }

  private async pump() {
    this.pumping = true;
    try {
      while (this.queue.length && !this.closed) {
        if (this.idle()) {
          await this.write(PULSE);
          this.ready = this.now() + WAKE_GAP;
        }
        const wait = this.ready - this.now();
        if (wait > 0) await sleep(wait);
        await this.write(this.queue.shift()!);
      }
    } catch (e) {
      this.toUi({ t: 'note', text: `write failed: ${(e as Error).message}` });
      this.queue = [];
    } finally {
      this.pumping = false;
    }
  }

  private error(kind: string, text: string, target: number | null = null) {
    this.toDecoder({ t: 'error', ts: this.ts(), kind, text, target });
  }

  /** Send `data` on `port` (refused beyond the 1443-byte limit, §2.4). */
  send(port: number, data: Uint8Array) {
    if (data.length > MAX_PORT_DATA) {
      return this.error(
        'send_oversize',
        `send: ${data.length} bytes for port ${port} exceed the ${MAX_PORT_DATA}-byte limit (protocol.md §2.4); not sent`,
      );
    }
    this.enqueue(wirePort(port, data));
  }

  /** Answer a pending semihost stdin read. */
  stdin(line: Uint8Array) {
    if (this.stdinWanted === null) return this.toUi({ t: 'note', text: 'no semihost stdin read is pending' });
    const r = this.semihost.stdinReply(line, this.stdinWanted);
    this.stdinWanted = null;
    this.enqueue(wirePort(PORTS.SEMIHOST, r));
  }

  get stdinPending() {
    return this.stdinWanted !== null;
  }

  /** Feed received bytes. */
  onData(d: Uint8Array) {
    this.rxBytes += d.length;
    this.lastRx = this.now();
    if (this.liveness !== 'alive') {
      this.liveness = 'alive';
      this.toUi({ t: 'liveness', state: 'alive' });
    }
    for (const e of this.scanner.feed(d)) {
      if ('oversize' in e) {
        this.error('framing_oversize', `framing: frame exceeded ${this.scanner.maxFrame} bytes, discarded ${e.oversize} bytes`);
      } else this.onFrame(e.frame);
    }
  }

  private onFrame(enc: Uint8Array) {
    let p: Uint8Array;
    try {
      p = cobsDecode(enc);
    } catch {
      return this.error('framing_invalid', `framing: invalid COBS (${enc.length} bytes) near ${hex(enc.subarray(0, 16))}…`);
    }
    const m = muxDecode(p);
    switch (m.kind) {
      case 'pulse':
        return;
      case 'short':
        return this.error('short_log', `decode: log frame too short (${m.payload.length} bytes)`);
      case 'log':
        if (m.type === TYPE_RES) return this.error('reserved_type', `decode: reserved frame type 2 [${hex(p, 32)}]`, m.target);
        return this.toDecoder({ t: 'log', ts: this.ts(), target: m.target, addr: m.addr, args: m.args.slice() });
      case 'port':
        return this.onPort(m.port, m.data.slice());
    }
  }

  private onPort(port: number, data: Uint8Array) {
    switch (port) {
      case PORTS.SEMIHOST:
        this.shChain = this.shChain.then(() => this.onSemihost(data));
        return;
      case PORTS.DEVICE_INFO:
        return this.toDecoder({ t: 'info', ts: this.ts(), data });
      case PORTS.PLOT:
        return this.toDecoder({ t: 'plot', data });
      case PORTS.FW:
      case PORTS.FW_LEGACY:
        if (this.fwQueue) {
          this.fwQueue.push({ port, data });
          this.fwWake?.();
          return;
        }
        return this.toUi({ t: 'port', port, data });
      case PORTS.COMMANDS:
      case PORTS.PRISM:
      case PORTS.TERMINAL:
        return this.toUi({ t: 'port', port, data });
      default: {
        const n = (this.unknownPorts.get(port) ?? 0) + 1;
        this.unknownPorts.set(port, n);
        if (n === 1) this.error('unknown_port', `port ${port}: no handler (${data.length} bytes)`);
        this.toUi({ t: 'port', port, data });
      }
    }
  }

  private async onSemihost(req: Uint8Array) {
    const op = req[0];
    const o =
      op !== undefined && isTimeOp(op) && req.length === 1
        ? { reply: this.semihost.timeReply(op) }
        : await this.semihost.handle(req);
    if (this.closed) return;
    if ('stdin' in o) {
      this.stdinWanted = o.stdin;
      return this.toUi({ t: 'stdin-wanted' });
    }
    if (o.console) this.toUi({ t: 'console', fid: o.console.fid, data: o.console.data });
    if (o.reply) this.enqueue(wirePort(PORTS.SEMIHOST, o.reply));
  }

  /** Run a FW update; port-59/3 frames go to it instead of the UI meanwhile. */
  async fwUpdate(image: Uint8Array, progress: (p: FwProgress) => void, signal?: AbortSignal): Promise<number> {
    if (this.fwQueue) throw new Error('a FW update is already running');
    this.fwQueue = [];
    const io: FwIo = {
      send: (port, data) => this.send(port, data),
      next: async (ms, sig) => {
        const end = this.now() + ms;
        while (!this.fwQueue!.length) {
          if (this.closed) throw new Error('session closed');
          if (sig?.aborted) return undefined;
          const left = end - this.now();
          if (left <= 0) return undefined;
          await new Promise<void>((r) => {
            const t = setTimeout(r, left);
            this.fwWake = () => {
              clearTimeout(t);
              r();
            };
            sig?.addEventListener('abort', () => this.fwWake?.(), { once: true });
          });
          this.fwWake = null;
        }
        return this.fwQueue!.shift();
      },
    };
    try {
      return await fwUpdate(io, image, progress, signal);
    } finally {
      this.fwQueue = null;
    }
  }
}
