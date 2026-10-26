// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Admission to a hub (§8, D14), shared by the browser's WebSocket link and the VS Code extension's
 * TCP link. A hub that accepts a client sends at least a pulse every 0.5 s, so a connection that
 * is open but silent is waiting for the operator's approval; a busy hub closes it.
 */

export type LinkState = 'connecting' | 'approval' | 'open' | 'busy' | 'closed' | 'error' | 'timeout';

/** Silence for this long after connecting: show "waiting for hub approval". */
export const APPROVAL_HINT_MS = 1000;
/** The hub's operator has this long to answer (the hub's own limit). */
export const APPROVAL_TIMEOUT_MS = 60_000;
export const HUB_BUSY = 'hub busy: another client is connected';

/** The link's state from connecting until the hub's first bytes, then until it ends. Each state
 * is reported once, and nothing after the end. */
export class HubAdmission {
  private got = false;
  private done = false;
  private timers: ReturnType<typeof setTimeout>[] = [];

  constructor(private onState: (s: LinkState, detail?: string) => void) {
    onState('connecting');
  }

  /** The hub has sent something: it accepted us. */
  get admitted(): boolean {
    return this.got;
  }

  get ended(): boolean {
    return this.done;
  }

  /** The connection is open: wait for the hub. `close` closes it when the approval times out. */
  opened(close: () => void, timeoutMs = APPROVAL_TIMEOUT_MS) {
    this.timers.push(
      setTimeout(() => !this.got && !this.done && this.onState('approval'), APPROVAL_HINT_MS),
      setTimeout(() => {
        if (this.got) return;
        this.end('timeout', `no data from the hub within ${timeoutMs / 1000} s (approval timed out?)`);
        close();
      }, timeoutMs),
    );
  }

  /** Bytes from the hub. */
  data() {
    if (this.got) return;
    this.got = true;
    this.clearTimers();
    if (!this.done) this.onState('open');
  }

  /** The link ended: report `s`, or nothing (our own close). Later calls are ignored. */
  end(s?: LinkState, detail?: string) {
    if (this.done) return;
    this.done = true;
    this.clearTimers();
    if (s) this.onState(s, detail);
  }

  private clearTimers() {
    this.timers.forEach(clearTimeout);
    this.timers = [];
  }
}
