// Host services over MessagePorts: semihost files and the resolver's cache, fetch and local lookup.

import { describe, expect, it } from 'vitest';
import { FsError, hex, MemoryFs, Resolver, type FsFile } from '@uclog/core';
import { HostFsProvider, hostResolverParts, RpcClient, serveHost, type HostFetchReply, type HostFs, type HostServices } from '../src/rpc.ts';
import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const sample = new Uint8Array(readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), '../../../../spec/vectors/sample.logdata')));

/** A HostFs over MemoryFs with numeric handles (what the VS Code host does with Node's fs). */
function hostFs(): HostFs {
  const m = new MemoryFs('mem');
  const files = new Map<number, FsFile>();
  let next = 1;
  const f = (fd: number) => {
    const x = files.get(fd);
    if (!x) throw new FsError('EBADF');
    return x;
  };
  return {
    open: async (p, fl) => {
      files.set(next, await m.open(p, fl));
      return next++;
    },
    read: (fd, n) => f(fd).read(n),
    write: (fd, d) => f(fd).write(d),
    close: (fd) => f(fd).close(),
    stat: (fd) => f(fd).stat(),
    mkdir: (p) => m.mkdir(p),
  };
}

function pair(s: HostServices) {
  const ch = new MessageChannel();
  serveHost(ch.port2, s);
  return new RpcClient(ch.port1);
}

describe('host services', () => {
  it('semihost files, with errno across the port', async () => {
    const fs = new HostFsProvider(pair({ fs: hostFs() }), 'workspace');
    const w = await fs.open(['a.txt'], { read: false, write: true, append: false, create: true, truncate: true, exclusive: false });
    expect(await w.write(new TextEncoder().encode('hi'))).toBe(2);
    await w.close();
    const r = await fs.open(['a.txt'], { read: true, write: false, append: false, create: false, truncate: false, exclusive: false });
    expect(new TextDecoder().decode(await r.read(10))).toBe('hi');
    expect((await r.stat()).size).toBe(2);
    const err = await fs.open(['nope'], { read: true, write: false, append: false, create: false, truncate: false, exclusive: false }).catch((e) => e);
    expect(err).toBeInstanceOf(FsError);
    expect(err.errno).toBe('ENOENT');
  });

  it('resolver: the host cache, then a fetch the host makes, then a local build', async () => {
    const hash = new Uint8Array(64).fill(0x5a);
    const h = hex(hash);
    const cache = new Map<string, Uint8Array>();
    const urls: string[] = [];
    let remote: Uint8Array | null = null;
    let local: Uint8Array | null = null;
    const rpc = pair({
      cache: { get: async (k) => cache.get(k), put: async (k, d) => void cache.set(k, d) },
      fetch: async (url): Promise<HostFetchReply> => {
        urls.push(url);
        return remote ? { status: 200, headers: {}, body: remote } : { status: 404, headers: { 'content-type': 'application/problem+json' }, body: new Uint8Array() };
      },
      sibling: async () => (local ? { name: 'build/app.logdata', data: local } : undefined),
    });
    const p = hostResolverParts(rpc, { cache: true, fetch: true, sibling: true });
    const res = new Resolver({ domain: 'acme', url: 'https://svc.example' }, p.cache, p.fetcher, p.sibling);

    expect((await res.resolve(hash)).source).toBeNull();
    expect(urls).toEqual([`https://svc.example/logdata/acme/${h}`]);

    local = sample;
    expect((await res.resolve(hash)).source).toEqual({ kind: 'sibling', name: 'build/app.logdata' });

    remote = sample;
    expect((await res.resolve(hash)).source?.kind).toBe('remote');
    expect(cache.has(h)).toBe(true);
    expect((await res.resolve(hash)).source?.kind).toBe('cache');
  });

  it('an unknown op is an error, not a hang', async () => {
    const ch = new MessageChannel();
    serveHost(ch.port2, {});
    await expect(new RpcClient(ch.port1).call('nope')).rejects.toThrow(/unknown host op nope/);
  });
});
