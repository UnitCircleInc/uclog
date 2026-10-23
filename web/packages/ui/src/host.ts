// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Hooks for an embedding host that is more than a web page (VS Code, plan 03 §3): its own connect
 * UI, file pickers, saving, settings and services. Set `UcLog.host` before the element starts.
 */

import type { HostServices } from '@uclog/workers';
import type { Settings } from './settings.ts';

export interface SettingsBackend {
  /** The stored settings (merged over the defaults). */
  load(): unknown;
  save(s: Settings): void;
}

export interface UcHost {
  /** Logdata cache and fetch, local builds, semihost files. */
  services?: HostServices;
  /** Shown instead of the Connect dialog. */
  showConnect?(): void;
  /** A file chosen by the user (null: cancelled). */
  pickFile?(kind: 'logdata' | 'firmware'): Promise<{ name: string; data: Uint8Array } | null>;
  /** Open a log record's source (`file:line` click-through). */
  openSource?(file: string, line: string): void;
  /** Save an export (instead of a browser download). */
  save?(name: string, data: Uint8Array): void;
  /** Shown instead of the Settings dialog. */
  openSettings?(): void;
  settings?: SettingsBackend;
  /** Semihost files served by `services.fs`, shown under this name (null: none). */
  files?: string | null;
  /** How the user changes the semihost folder (shown in the Device view). */
  filesHint?: string;
}
