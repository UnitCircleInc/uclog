// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

// The one-file app (uclog-app-N.html, `vite build --mode single`): the workers and the sample
// logdata inlined, for opening from disk. Pages opened from files share one origin, so its
// settings use their own storage key.

import TransportWorker from '@uclog/workers/transport.worker?worker&inline';
import DecodeWorker from '@uclog/workers/decode.worker?worker&inline';
import { UcLog } from '@uclog/ui';
import sample from '../../../../spec/vectors/sample.logdata?url'; // lib mode inlines it (a data: URL)
import { startPage } from './page.ts';

UcLog.workers = {
  transport: () => new TransportWorker({ name: 'uclog-transport' }),
  decode: () => new DecodeWorker({ name: 'uclog-decode' }),
};
startPage('uclog-app-file', sample);
