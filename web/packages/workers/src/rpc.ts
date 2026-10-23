// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Services an embedding host provides to the workers (VS Code: the extension host fetches and
 * caches logdata, finds it in the workspace, and serves semihost files; plan 03 §3). A worker asks
 * the main thread over a MessagePort; the main thread calls the host.
 */

import { FsError, type Errno, type FsFile, type FsProvider, type LogdataCache, type OpenFlags, type SiblingLookup } from '@uclog/core';

/** Semihost files on the host. Errors: `FsError` (or any error with an `errno` name). */
export interface HostFs {
  open(path: string[], flags: OpenFlags): Promise<number>;
  read(fd: number, n: number): Promise<Uint8Array>;
  write(fd: number, data: Uint8Array): Promise<number>;
  close(fd: number): Promise<void>;
  stat(fd: number): Promise<{ size: number; mtime: number; dir: boolean }>;
  mkdir(path: string[]): Promise<void>;
}

export interface HostFetchReply {
  status: number;
  headers: Record<string, string>;
  body: Uint8Array;
}

export interface HostServices {
  cache?: LogdataCache;
  /** GET without CORS (the host adds the token if it keeps it). Follows redirects. */
  fetch?(url: string, headers: Record<string, string>): Promise<HostFetchReply>;
  sibling?: SiblingLookup;
  fs?: HostFs;
}

/** What the decode worker may ask for. */
export interface HostCaps {
  cache: boolean;
  fetch: boolean;
  sibling: boolean;
}

export function hostCaps(s: HostServices): HostCaps {
  return { cache: !!s.cache, fetch: !!s.fetch, sibling: !!s.sibling };
}

interface Request {
  id: number;
  op: string;
  args: unknown[];
}
interface Reply {
  id: number;
  ok: boolean;
  value?: unknown;
  error?: string;
  errno?: Errno;
}

export class RpcClient {
  private nextId = 1;
  private waiting = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();

  constructor(private port: MessagePort) {
    port.onmessage = (e: MessageEvent<Reply>) => {
      const r = e.data;
      const w = this.waiting.get(r.id);
      this.waiting.delete(r.id);
      if (!w) return;
      if (r.ok) w.resolve(r.value);
      else w.reject(r.errno ? new FsError(r.errno) : new Error(r.error));
    };
  }

  call<T>(op: string, ...args: unknown[]): Promise<T> {
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      this.waiting.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.port.postMessage({ id, op, args } satisfies Request);
    });
  }
}

/** Answer requests on `port` with the host's services (main thread). */
export function serveHost(port: MessagePort, s: HostServices) {
  const ops: Record<string, (...a: never[]) => Promise<unknown>> = {
    'cache-get': (h: string) => s.cache!.get(h),
    'cache-put': (h: string, d: Uint8Array) => s.cache!.put(h, d),
    fetch: (url: string, headers: Record<string, string>) => s.fetch!(url, headers),
    sibling: (h: string) => s.sibling!(h),
    'fs-open': (p: string[], f: OpenFlags) => s.fs!.open(p, f),
    'fs-read': (fd: number, n: number) => s.fs!.read(fd, n),
    'fs-write': (fd: number, d: Uint8Array) => s.fs!.write(fd, d),
    'fs-close': (fd: number) => s.fs!.close(fd),
    'fs-stat': (fd: number) => s.fs!.stat(fd),
    'fs-mkdir': (p: string[]) => s.fs!.mkdir(p),
  };
  port.onmessage = async (e: MessageEvent<Request>) => {
    const { id, op, args } = e.data;
    try {
      const f = ops[op];
      if (!f) throw new Error(`unknown host op ${op}`);
      port.postMessage({ id, ok: true, value: await f(...(args as never[])) } satisfies Reply);
    } catch (err) {
      const errno = (err as { errno?: unknown }).errno;
      port.postMessage({
        id,
        ok: false,
        error: (err as Error).message ?? String(err),
        errno: typeof errno === 'string' ? (errno as Errno) : undefined,
      } satisfies Reply);
    }
  };
}

/** The decode worker's view of the host: a logdata cache, a fetch and the local-build lookup. */
export function hostResolverParts(rpc: RpcClient, caps: HostCaps) {
  const cache: LogdataCache | null = caps.cache
    ? { get: (h) => rpc.call<Uint8Array | undefined>('cache-get', h), put: (h, d) => rpc.call<void>('cache-put', h, d) }
    : null;
  const fetcher: typeof fetch | null = caps.fetch
    ? async (input, init) => {
        const headers: Record<string, string> = {};
        new Headers(init?.headers).forEach((v, k) => (headers[k] = v));
        const r = await rpc.call<HostFetchReply>('fetch', String(input instanceof Request ? input.url : input), headers);
        return new Response(r.status === 204 || r.status === 304 ? null : (r.body as BodyInit), { status: r.status, headers: r.headers });
      }
    : null;
  const sibling: SiblingLookup | null = caps.sibling ? (h) => rpc.call('sibling', h) : null;
  return { cache, fetcher, sibling };
}

/** Semihost files served by the host, for the transport worker. */
export class HostFsProvider implements FsProvider {
  constructor(
    private rpc: RpcClient,
    readonly name: string,
  ) {}

  mkdir(path: string[]) {
    return this.rpc.call<void>('fs-mkdir', path);
  }

  async open(path: string[], flags: OpenFlags): Promise<FsFile> {
    const fd = await this.rpc.call<number>('fs-open', path, flags);
    const rpc = this.rpc;
    return {
      read: (n) => rpc.call('fs-read', fd, n),
      write: (d) => rpc.call('fs-write', fd, d),
      close: () => rpc.call('fs-close', fd),
      stat: () => rpc.call('fs-stat', fd),
    };
  }
}
