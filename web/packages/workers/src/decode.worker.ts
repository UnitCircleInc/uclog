// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/// <reference lib="webworker" />
import { DecodeHost } from './decode-host.ts';
import { IdbCache } from './idb.ts';
import type { MainToDecode, TransportToDecoder } from './protocol.ts';

const scope = self as unknown as DedicatedWorkerGlobalScope;
const host = new DecodeHost((m, transfer) => scope.postMessage(m, transfer ?? []), typeof indexedDB === 'undefined' ? null : new IdbCache());
scope.onmessage = (e: MessageEvent<MainToDecode>) => {
  const m = e.data;
  if (m.t === 'init') {
    const port = m.transport;
    port.onmessage = (ev: MessageEvent<TransportToDecoder>) => host.onTransport(ev.data, (a) => port.postMessage(a));
  }
  host.handle(m);
};
