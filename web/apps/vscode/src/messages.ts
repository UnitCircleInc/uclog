// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** Messages between the extension host and the logger webview (plan 03 §2, "Data path"). */

export type LinkState = 'connecting' | 'approval' | 'open' | 'busy' | 'timeout' | 'closed' | 'error';

/** Passed in the page (no round trip before the element starts). */
export interface WebviewInit {
  settings: unknown;
  /** Semihost root shown in the Device view (null: none). */
  files: string | null;
  terminalForward: boolean;
}

export type ToWebview =
  /** Connect the logger to link `id` (the host opens it when the webview asks). */
  | { t: 'connect'; id: number; description: string }
  | { t: 'disconnect' }
  | { t: 'link'; id: number; state: LinkState; detail?: string }
  | { t: 'rx'; id: number; data: Uint8Array }
  | { t: 'settings'; settings: unknown }
  | { t: 'files'; files: string | null }
  | { t: 'load-logdata'; name: string; data: Uint8Array }
  | { t: 'fw'; name: string; data: Uint8Array }
  /** Port-61 output to the host too (the pseudoterminal is open). */
  | { t: 'terminal-forward'; on: boolean }
  | { t: 'send'; port: number; data: Uint8Array }
  /** Shown in the logger's status line and Device view notes. */
  | { t: 'note'; text: string }
  | { t: 'reply'; id: number; ok: boolean; value?: unknown; error?: string; errno?: string };

export interface ConnSummary {
  link: string;
  description?: string;
  detail?: string;
  liveness: string | null;
}

export type FromWebview =
  | { t: 'ready' }
  | { t: 'open'; id: number }
  | { t: 'tx'; id: number; data: Uint8Array }
  | { t: 'close'; id: number }
  | { t: 'state'; conn: ConnSummary }
  | { t: 'terminal'; data: Uint8Array }
  | { t: 'settings-save'; settings: unknown }
  | { t: 'show-connect' }
  | { t: 'open-settings' }
  | { t: 'token-refused' }
  | { t: 'open-source'; file: string; line: string }
  | { t: 'save'; name: string; data: Uint8Array }
  /** Services and pickers that answer (`reply`). */
  | { t: 'call'; id: number; op: string; args: unknown[] };
