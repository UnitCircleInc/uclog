// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * WebSocket link to a `uclog hub` (§4.5, §8). Opened inside the transport worker. Close code 4001
 * means "hub busy"; an open but silent connection is waiting for the operator's approval
 * (`HubAdmission`).
 */

import { HUB_BUSY, HubAdmission, type LinkState } from '@uclog/core';

export const WS_BUSY = 4001;

export class WebSocketLink {
  readonly hub = true;
  private ws: WebSocket;
  private adm: HubAdmission;

  constructor(
    readonly url: string,
    private onData: (b: Uint8Array) => void,
    onState: (s: LinkState, detail?: string) => void,
  ) {
    this.ws = new WebSocket(url);
    this.ws.binaryType = 'arraybuffer';
    this.adm = new HubAdmission(onState);
    this.ws.onopen = () => this.adm.opened(() => this.ws.close());
    this.ws.onmessage = (e) => {
      if (typeof e.data === 'string') return; // protocol error; ignored
      this.adm.data();
      this.onData(new Uint8Array(e.data as ArrayBuffer));
    };
    this.ws.onclose = (e) => {
      if (e.code === WS_BUSY) this.adm.end('busy', HUB_BUSY);
      else if (!this.adm.admitted && e.code !== 1000) this.adm.end('error', `connection closed (${e.code}${e.reason ? ` ${e.reason}` : ''})`);
      else this.adm.end('closed', e.reason || undefined);
    };
    this.ws.onerror = () => {
      /* onclose follows with the code */
    };
  }

  write(b: Uint8Array) {
    if (this.ws.readyState === WebSocket.OPEN) this.ws.send(b);
  }

  /** Close from our side ("closed" is reported when the socket has closed). */
  close() {
    this.ws.close(1000);
  }
}
