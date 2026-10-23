// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * The `<uc-log>` bundle (plan 02 §7). The two workers are inlined (blob: URLs, so the host's CSP
 * needs `worker-src blob:`), unless `worker-base` points at a directory with
 * `uclog-transport-worker.js` and `uclog-decode-worker.js` (built into `dist/workers/`).
 */

import TransportWorker from '@uclog/workers/transport.worker?worker&inline';
import DecodeWorker from '@uclog/workers/decode.worker?worker&inline';
import { UcLog } from '@uclog/ui';

UcLog.workers = {
  transport: () => new TransportWorker({ name: 'uclog-transport' }),
  decode: () => new DecodeWorker({ name: 'uclog-decode' }),
};

export * from '@uclog/ui';
export { Client } from '@uclog/workers';
export type { LinkSpec, ConnectionState, DecodeState, PlotData } from '@uclog/workers';
