// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** Workers as separate module files, for apps built with Vite (the demo). The element bundle
 * inlines them instead. */

import type { WorkerFactories } from './client.ts';

export const defaultWorkers: WorkerFactories = {
  transport: () => new Worker(new URL('./transport.worker.ts', import.meta.url), { type: 'module', name: 'uclog-transport' }),
  decode: () => new Worker(new URL('./decode.worker.ts', import.meta.url), { type: 'module', name: 'uclog-decode' }),
};
