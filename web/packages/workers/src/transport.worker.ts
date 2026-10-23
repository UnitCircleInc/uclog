// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/// <reference lib="webworker" />
import type { MainToTransport } from './protocol.ts';
import { TransportHost } from './transport-host.ts';

const scope = self as unknown as DedicatedWorkerGlobalScope;
const host = new TransportHost((m, transfer) => scope.postMessage(m, transfer ?? []));
scope.onmessage = (e: MessageEvent<MainToTransport>) => host.handle(e.data);
