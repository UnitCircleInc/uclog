// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** Log records and error records (protocol.md §7, plans README §5.6). */

export interface LogRecord {
  /** Sequence number, unique per client. */
  seq: number;
  /** Seconds since the session started (client receive time, D16). */
  ts: number;
  target: number | null;
  level: string;
  file: string;
  line: string;
  text: string;
  /** Error kind for error records (`framing_invalid`, `no_logdata`, …). */
  error?: string;
}

export const LEVEL_INDEX: Record<string, number> = { DEBUG: 0, INFO: 1, WARN: 2, ERROR: 3, FATAL: 4, PANIC: 5 };

export function levelIndex(level: string): number {
  return LEVEL_INDEX[level] ?? 3;
}

export const UCLOG_SOURCE = '<uclog>';
