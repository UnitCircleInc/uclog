// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * WebSocket link to a `uclog hub` (§4.5, §8). Opened inside the transport worker. Close code 4001
 * means "hub busy"; an open but silent connection is waiting for the operator's approval
 * (`HubAdmission`).
 */

import { HUB_BUSY, HubAdmission, type LinkState } from '@uclog/core';

export const WS_BUSY = 4001;

/** A loopback, private or link-local address, or a .local name: Chrome's "local network". */
export function isLocalHost(host: string): boolean {
  const h = host.replace(/^\[|\]$/g, '').toLowerCase();
  if (h === 'localhost' || h.endsWith('.localhost') || h.endsWith('.local')) return true;
  if (h === '::1' || h.startsWith('fe80:') || /^f[cd][0-9a-f]{2}:/.test(h)) return true;
  const m = /^(\d+)\.(\d+)\.\d+\.\d+$/.exec(h);
  if (!m) return false;
  const [a, b] = [Number(m[1]), Number(m[2])];
  return a === 127 || a === 10 || (a === 172 && b >= 16 && b < 32) || (a === 192 && b === 168) || (a === 169 && b === 254);
}

/** Why a connection from a public page (`pageHost`) to a hub on the local network may never open:
 * Chrome's Local Network Access needs the site's permission (it asks, or it was denied). */
export function localNetworkHint(url: string, pageHost: string): string {
  let host: string;
  try {
    host = new URL(url).hostname;
  } catch {
    return '';
  }
  return isLocalHost(host) && !isLocalHost(pageHost)
    ? ': the browser may be blocking this public page from the local network (allow "Local network access" for this site in its site settings)'
    : '';
}

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
    let opened = false;
    this.ws.onopen = () => {
      opened = true;
      this.adm.opened(() => this.ws.close());
    };
    this.ws.onmessage = (e) => {
      if (typeof e.data === 'string') return; // protocol error; ignored
      this.adm.data();
      this.onData(new Uint8Array(e.data as ArrayBuffer));
    };
    this.ws.onclose = (e) => {
      if (e.code === WS_BUSY) this.adm.end('busy', HUB_BUSY);
      else if (!this.adm.admitted && e.code !== 1000) {
        const hint = !opened && e.code === 1006 ? localNetworkHint(url, globalThis.location?.hostname ?? '') : '';
        this.adm.end('error', `connection closed (${e.code}${e.reason ? ` ${e.reason}` : ''})${hint}`);
      }
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
