// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * The transport worker's logic (plan 02 §3): it owns the Session's transport half and the link
 * (WebSocket and the simulated device live here; Serial/HID/BLE bytes are forwarded by the main
 * thread). Log frames go to the decode worker in batches; when it falls behind, log frames are
 * dropped (counted, with an error record), never port 63/59/61 traffic.
 */

import { LoopbackDevice, parseLoopbackOptions, TransportSession, type RawLink, type ToDecoder } from '@uclog/core';
import { DirectoryFs, WebSocketLink } from '@uclog/transports-web';
import type { DecoderToTransport, MainToTransport, TransportToDecoder, TransportToMain } from './protocol.ts';

/** Unacknowledged batches before log frames are dropped. */
export const MAX_OUTSTANDING = 64;
const FLUSH_MS = 16;
const MAX_BATCH = 4000;

type Post = (m: TransportToMain, transfer?: Transferable[]) => void;

export class TransportHost {
  private session: TransportSession | null = null;
  private closeLink: (() => void) | null = null;
  private decoder: MessagePort | null = null;
  private batch: ToDecoder[] = [];
  private flushTimer: ReturnType<typeof setTimeout> | undefined;
  private nextId = 1;
  private acked = 0;
  private dropped = 0;
  private droppedReported = 0;
  private fs: DirectoryFs | null = null;
  private fwAbort: AbortController | null = null;
  private statsTimer: ReturnType<typeof setInterval>;
  private gen = 0;

  constructor(private post: Post) {
    this.statsTimer = setInterval(() => {
      if (this.session) this.post({ t: 'stats', rx: this.session.rxBytes, tx: this.session.txBytes, dropped: this.dropped });
    }, 1000);
  }

  dispose() {
    clearInterval(this.statsTimer);
    this.disconnect();
  }

  private toDecoder(m: ToDecoder) {
    const outstanding = this.nextId - 1 - this.acked;
    if (m.t === 'log' && outstanding >= MAX_OUTSTANDING) {
      this.dropped++;
      return;
    }
    if (this.dropped > this.droppedReported && outstanding < MAX_OUTSTANDING / 2) {
      const n = this.dropped - this.droppedReported;
      this.droppedReported = this.dropped;
      this.batch.push({
        t: 'error',
        ts: m.t === 'log' || m.t === 'error' || m.t === 'info' ? m.ts : 0,
        kind: 'decoder_behind',
        text: `decode: the decoder fell behind; dropped ${n} log frames`,
        target: null,
      });
    }
    this.batch.push(m);
    if (this.batch.length >= MAX_BATCH) this.flush();
    else this.flushTimer ??= setTimeout(() => this.flush(), FLUSH_MS);
  }

  private flush() {
    clearTimeout(this.flushTimer);
    this.flushTimer = undefined;
    if (!this.batch.length || !this.decoder) return;
    const msg: TransportToDecoder = { t: 'batch', id: this.nextId++, items: this.batch };
    this.batch = [];
    this.decoder.postMessage(msg);
  }

  handle(m: MainToTransport) {
    switch (m.t) {
      case 'init':
        this.decoder = m.decoder;
        this.decoder.onmessage = (e: MessageEvent<DecoderToTransport>) => {
          if (e.data.t === 'ack') this.acked = Math.max(this.acked, e.data.id);
        };
        return;
      case 'connect':
        return this.connect(m.link, m.fs);
      case 'rx':
        return this.session?.onData(m.data);
      case 'link-closed':
        this.post({ t: 'link', state: 'closed', detail: m.reason });
        return this.stop();
      case 'send':
        return this.session?.send(m.port, m.data);
      case 'stdin':
        return this.session?.stdin(m.line);
      case 'disconnect':
        this.disconnect();
        return;
      case 'set-fs':
        this.fs = m.fs ? new DirectoryFs(m.fs) : null;
        void this.session?.semihost.setFs(this.fs);
        return;
      case 'fw-start':
        return void this.fw(m.image);
      case 'fw-cancel':
        this.fwAbort?.abort();
        return;
    }
  }

  private connect(link: import('./protocol.ts').LinkInit, fs: FileSystemDirectoryHandle | null) {
    this.stop();
    const gen = ++this.gen;
    this.fs = fs ? new DirectoryFs(fs) : null;
    this.dropped = this.droppedReported = 0;
    let raw: RawLink;
    let session: TransportSession;
    const mk = (r: RawLink) =>
      new TransportSession(
        r,
        (d) => this.toDecoder(d),
        (u) => this.post(u),
        { fs: this.fs },
      );
    switch (link.kind) {
      case 'ws': {
        let ws: WebSocketLink;
        raw = { hub: true, write: (b) => ws.write(b) };
        session = mk(raw);
        ws = new WebSocketLink(
          link.url,
          (b) => session.onData(b),
          (s, detail) => {
            if (gen !== this.gen) return;
            this.post({ t: 'link', state: s, detail, description: link.url });
            if (s === 'busy' || s === 'closed' || s === 'error' || s === 'timeout') this.stop();
          },
        );
        this.closeLink = () => ws.close();
        break;
      }
      case 'loopback': {
        let opts;
        try {
          opts = parseLoopbackOptions(link.opts);
        } catch (e) {
          this.post({ t: 'link', state: 'error', detail: `loopback: ${(e as Error).message}` });
          return;
        }
        let dev: LoopbackDevice;
        raw = { hub: false, write: (b) => dev.write(b) };
        session = mk(raw);
        dev = new LoopbackDevice(opts, (b) => queueMicrotask(() => gen === this.gen && session.onData(b)));
        this.closeLink = () => dev.close();
        this.post({ t: 'link', state: 'open', description: `loopback (${opts.board})` });
        dev.start();
        break;
      }
      case 'forward': {
        raw = {
          hub: false,
          write: (b) => {
            const c = b.slice();
            this.post({ t: 'tx', data: c }, [c.buffer]);
          },
        };
        session = mk(raw);
        this.closeLink = null;
        this.post({ t: 'link', state: 'open', description: link.description });
        break;
      }
    }
    this.session = session;
    session.open();
  }

  private stop() {
    this.fwAbort?.abort();
    this.session?.close();
    this.session = null;
    this.closeLink?.();
    this.closeLink = null;
    this.flush();
  }

  private disconnect() {
    const had = !!this.session;
    this.gen++;
    this.stop();
    if (had) this.post({ t: 'link', state: 'closed', detail: 'disconnected' });
  }

  private async fw(image: Uint8Array) {
    const s = this.session;
    if (!s) return this.post({ t: 'fw-done', error: 'not connected' });
    this.fwAbort = new AbortController();
    try {
      const port = await s.fwUpdate(image, (p) => this.post({ t: 'fw', progress: p }), this.fwAbort.signal);
      this.post({ t: 'fw-done', port });
    } catch (e) {
      this.post({ t: 'fw-done', error: (e as Error).message });
    } finally {
      this.fwAbort = null;
    }
  }
}
