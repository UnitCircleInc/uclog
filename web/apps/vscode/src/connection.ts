// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * A raw TCP client connection to a hub, in the extension host (no browser Origin rules, plan 03
 * §2). Like the web's WebSocket link: a hub that closes before sending anything is busy; one that
 * accepts but stays silent is asking its operator (D14, `HubAdmission`).
 */

import * as net from 'node:net';
import { HUB_BUSY, HubAdmission, type LinkState } from '@uclog/core';

export class HubConnection {
  private sock: net.Socket;
  private adm: HubAdmission;

  constructor(
    readonly host: string,
    readonly port: number,
    onState: (s: LinkState, detail?: string) => void,
    private onData: (b: Uint8Array) => void,
    o: { approvalTimeoutMs?: number } = {},
  ) {
    this.adm = new HubAdmission(onState);
    this.sock = net.connect({ host, port });
    this.sock.setNoDelay(true);
    this.sock.on('connect', () => this.adm.opened(() => this.sock.destroy(), o.approvalTimeoutMs));
    this.sock.on('data', (b: Buffer) => {
      this.adm.data();
      this.onData(new Uint8Array(b.buffer, b.byteOffset, b.length));
    });
    this.sock.on('error', (e: NodeJS.ErrnoException) => {
      // A busy hub's close can arrive as a reset when our first pulse was unread.
      if (!this.adm.admitted && e.code === 'ECONNRESET') this.end('busy', HUB_BUSY);
      else this.end('error', e.code === 'ECONNREFUSED' ? `no hub at ${host}:${port}` : e.message);
    });
    this.sock.on('close', () => {
      if (!this.adm.admitted) this.end('busy', HUB_BUSY);
      else this.end('closed', 'the hub closed the connection');
    });
  }

  private end(s: LinkState, detail?: string) {
    this.sock.destroy();
    this.adm.end(s, detail);
  }

  write(b: Uint8Array) {
    if (!this.adm.ended) this.sock.write(b);
  }

  /** Close from our side (no state is reported). */
  close() {
    this.adm.end();
    this.sock.end();
    this.sock.destroy();
  }
}
