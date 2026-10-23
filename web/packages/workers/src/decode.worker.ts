// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/// <reference lib="webworker" />
import { DecodeHost } from './decode-host.ts';
import { IdbCache } from './idb.ts';
import type { MainToDecode, TransportToDecoder } from './protocol.ts';
import { hostResolverParts, RpcClient } from './rpc.ts';

const scope = self as unknown as DedicatedWorkerGlobalScope;
const host = new DecodeHost((m, transfer) => scope.postMessage(m, transfer ?? []), typeof indexedDB === 'undefined' ? null : new IdbCache());
scope.onmessage = (e: MessageEvent<MainToDecode>) => {
  const m = e.data;
  if (m.t === 'init') {
    const port = m.transport;
    port.onmessage = (ev: MessageEvent<TransportToDecoder>) => host.onTransport(ev.data, (a) => port.postMessage(a));
    if (m.host && m.caps) {
      // The embedding host fetches, caches and finds logdata (VS Code).
      const p = hostResolverParts(new RpcClient(m.host), m.caps);
      if (p.cache) host.resolver.cache = p.cache;
      if (p.fetcher) host.resolver.fetcher = p.fetcher;
      host.resolver.sibling = p.sibling;
    }
  }
  host.handle(m);
};
