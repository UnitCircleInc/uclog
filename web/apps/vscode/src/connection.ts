// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * A raw TCP client connection to a hub, in the extension host (no browser Origin rules, plan 03
 * §2). Like the web's WebSocket link: a hub that closes before sending anything is busy; one that
 * accepts but stays silent is asking its operator (D14), for at most 60 s.
 */

import * as net from 'node:net';
import type { LinkState } from './messages.ts';

export const APPROVAL_HINT_MS = 1000;
export const APPROVAL_TIMEOUT_MS = 60_000;

export class HubConnection {
  private sock: net.Socket;
  private gotData = false;
  private ended = false;
  private hint: ReturnType<typeof setTimeout> | undefined;
  private timeout: ReturnType<typeof setTimeout> | undefined;

  constructor(
    readonly host: string,
    readonly port: number,
    private onState: (s: LinkState, detail?: string) => void,
    private onData: (b: Uint8Array) => void,
    o: { approvalTimeoutMs?: number } = {},
  ) {
    this.onState('connecting');
    this.sock = net.connect({ host, port });
    this.sock.setNoDelay(true);
    this.sock.on('connect', () => {
      this.hint = setTimeout(() => !this.gotData && this.onState('approval'), APPROVAL_HINT_MS);
      this.timeout = setTimeout(() => {
        if (!this.gotData) this.end('timeout', `no data from the hub within ${(o.approvalTimeoutMs ?? APPROVAL_TIMEOUT_MS) / 1000} s (approval timed out?)`);
      }, o.approvalTimeoutMs ?? APPROVAL_TIMEOUT_MS);
    });
    this.sock.on('data', (b: Buffer) => {
      if (!this.gotData) {
        this.gotData = true;
        this.clearTimers();
        this.onState('open');
      }
      this.onData(new Uint8Array(b.buffer, b.byteOffset, b.length));
    });
    this.sock.on('error', (e: NodeJS.ErrnoException) => {
      // A busy hub's close can arrive as a reset when our first pulse was unread.
      if (!this.gotData && e.code === 'ECONNRESET') this.end('busy', 'hub busy: another client is connected');
      else this.end('error', e.code === 'ECONNREFUSED' ? `no hub at ${host}:${port}` : e.message);
    });
    this.sock.on('close', () => {
      if (!this.gotData) this.end('busy', 'hub busy: another client is connected');
      else this.end('closed', 'the hub closed the connection');
    });
  }

  private clearTimers() {
    clearTimeout(this.hint);
    clearTimeout(this.timeout);
  }

  private end(s: LinkState, detail?: string) {
    if (this.ended) return;
    this.ended = true;
    this.clearTimers();
    this.sock.destroy();
    this.onState(s, detail);
  }

  write(b: Uint8Array) {
    if (!this.ended) this.sock.write(b);
  }

  /** Close from our side (no state is reported). */
  close() {
    this.ended = true;
    this.clearTimers();
    this.sock.end();
    this.sock.destroy();
  }
}
