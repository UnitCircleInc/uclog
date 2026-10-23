// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * WebSocket link to a `uclog hub` (§4.5, §8). Opened inside the transport worker. Close code 4001
 * means "hub busy"; an open but silent connection is waiting for the operator's approval (60 s).
 */

export const WS_BUSY = 4001;
export const APPROVAL_TIMEOUT = 60_000;

export type WsState = 'connecting' | 'approval' | 'open' | 'busy' | 'closed' | 'error' | 'timeout';

export class WebSocketLink {
  readonly hub = true;
  private ws: WebSocket;
  private gotData = false;
  private approvalTimer: ReturnType<typeof setTimeout> | undefined;
  private approvalHint: ReturnType<typeof setTimeout> | undefined;

  constructor(
    readonly url: string,
    private onData: (b: Uint8Array) => void,
    private onState: (s: WsState, detail?: string) => void,
  ) {
    this.ws = new WebSocket(url);
    this.ws.binaryType = 'arraybuffer';
    this.onState('connecting');
    this.ws.onopen = () => {
      // The hub sends at least a pulse every 0.5 s once it accepts us; silence means it is asking
      // its operator (D14).
      this.approvalHint = setTimeout(() => !this.gotData && this.onState('approval'), 1000);
      this.approvalTimer = setTimeout(() => {
        if (!this.gotData) {
          this.onState('timeout', 'no data from the hub within 60 s (approval timed out?)');
          this.ws.close();
        }
      }, APPROVAL_TIMEOUT);
    };
    this.ws.onmessage = (e) => {
      if (typeof e.data === 'string') return; // protocol error; ignored
      if (!this.gotData) {
        this.gotData = true;
        clearTimeout(this.approvalHint);
        clearTimeout(this.approvalTimer);
        this.onState('open');
      }
      this.onData(new Uint8Array(e.data as ArrayBuffer));
    };
    this.ws.onclose = (e) => {
      clearTimeout(this.approvalHint);
      clearTimeout(this.approvalTimer);
      if (e.code === WS_BUSY) this.onState('busy', 'hub busy: another client is connected');
      else if (!this.gotData && e.code !== 1000) this.onState('error', `connection closed (${e.code}${e.reason ? ` ${e.reason}` : ''})`);
      else this.onState('closed', e.reason || undefined);
    };
    this.ws.onerror = () => {
      /* onclose follows with the code */
    };
  }

  write(b: Uint8Array) {
    if (this.ws.readyState === WebSocket.OPEN) this.ws.send(b);
  }

  close() {
    this.ws.close(1000);
  }
}
