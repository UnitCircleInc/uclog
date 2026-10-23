// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

// The standalone page: one full-size <uc-log>. Query parameters set its attributes
// (?hub-url=ws://host:9000&theme=dark&layout=grid&logdata-domain=…).

import { UcLog } from '@uclog/ui';
import { defaultWorkers } from '@uclog/workers/default-workers';
import sampleUrl from '../../../../spec/vectors/sample.logdata?url';

UcLog.workers = defaultWorkers;
const el = document.createElement('uc-log');
el.setAttribute('storage-key', 'uclog-demo');
const q = new URLSearchParams(location.search);
for (const a of ['hub-url', 'theme', 'layout', 'panels', 'logdata-domain', 'logdata-url']) {
  const v = q.get(a);
  if (v !== null) el.setAttribute(a, v);
}

// The simulated device logs with spec/vectors/sample.logdata. Load it under the simulated
// device's app_hash (5a × 64) so it is only used for that device.
void fetch(sampleUrl)
  .then((r) => r.arrayBuffer())
  .then((b) => el.loadLogdata({ name: `${'5a'.repeat(64)}.logdata`, data: new Uint8Array(b) }))
  .catch(() => {});

const auto = q.get('connect');
if (auto) void el.connect(auto);

document.body.append(el);
Object.assign(window, { ucLog: el });
