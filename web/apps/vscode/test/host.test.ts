// Extension-host modules without VS Code: hub binary and process, TCP link states, files, fetch.

import { describe, expect, it } from 'vitest';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, statSync, symlinkSync, writeFileSync, chmodSync } from 'node:fs';
import * as net from 'node:net';
import { homedir, tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { bundledHubPath, deviceProblem, listDevices, parseHubAddress, PrivateHub, resolveHub } from '../src/hub.ts';
import { HubConnection } from '../src/connection.ts';
import { findLocalLogdata, ImageHashes, imageHash, LogdataStore, NodeFs, sharedCacheDir } from '../src/files.ts';
import { allowedUrl, hostFetch } from '../src/fetch.ts';
import type { LinkState } from '../src/messages.ts';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../../../..');
const HUB = join(repo, 'native/target', existsSync(join(repo, 'native/target/release/uclog-hub')) ? 'release' : 'debug', 'uclog-hub');
const tmp = () => mkdtempSync(join(tmpdir(), 'uclog-host-'));
const RW = { read: true, write: true, append: false, create: true, truncate: false, exclusive: false };
const RO = { read: true, write: false, append: false, create: false, truncate: false, exclusive: false };

describe('hub binary', () => {
  it('picks bin/<platform>-<arch>', () => {
    expect(bundledHubPath('/ext', 'win32', 'arm64')).toBe(join('/ext', 'bin', 'win32-arm64', 'uclog-hub.exe'));
    expect(bundledHubPath('/ext', 'linux', 'arm')).toBe(join('/ext', 'bin', 'linux-arm', 'uclog-hub'));
  });

  it('sets the exec bit dropped by the VSIX unzip', async () => {
    const d = tmp();
    const p = bundledHubPath(d, 'linux', 'x64');
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, '#!/bin/sh\n');
    chmodSync(p, 0o644);
    expect(await resolveHub(d, '', 'linux', 'x64')).toBe(p);
    expect(statSync(p).mode & 0o111).toBe(0o111);
  });

  it('explains a missing binary', async () => {
    await expect(resolveHub(tmp(), '', 'linux', 'riscv64')).rejects.toThrow(/no bundled uclog-hub for linux-riscv64; set uclog.hubPath/);
    await expect(resolveHub(tmp(), '/nope/uclog-hub')).rejects.toThrow(/uclog.hubPath: \/nope\/uclog-hub does not exist/);
  });

  it('parses hub addresses', () => {
    expect(parseHubAddress('tcp://lab.local:9001')).toEqual({ host: 'lab.local', port: 9001 });
    expect(parseHubAddress('10.0.0.5')).toEqual({ host: '10.0.0.5', port: 9000 });
    expect(parseHubAddress('[fe80::1]:9002')).toEqual({ host: 'fe80::1', port: 9002 });
    expect(() => parseHubAddress('ws://x:9000')).toThrow(/tcp:\/\//);
    expect(() => parseHubAddress('x:99999')).toThrow(/bad port/);
  });
});

describe.skipIf(!existsSync(HUB))('uclog-hub', () => {
  it('lists devices as JSON', async () => {
    const { devices } = await listDevices(HUB, { hubs: 0 });
    expect(Array.isArray(devices)).toBe(true);
  });

  it('starts a private hub, serves a client, and stops', async () => {
    const lines: string[] = [];
    const h = await PrivateHub.start(HUB, 'loopback:rate=5', { log: (l) => lines.push(l) });
    expect(h.port).toBeGreaterThan(0);
    const states: LinkState[] = [];
    let bytes = 0;
    const c = new HubConnection('127.0.0.1', h.port, (s) => states.push(s), (b) => (bytes += b.length));
    await expect.poll(() => bytes, { timeout: 5000 }).toBeGreaterThan(0);
    expect(states).toEqual(['connecting', 'open']);
    c.close();
    await h.stop();
    expect(h.exited).toBe(true);
    expect(lines.join('\n')).toContain('loopback');
  });

  it('keeps retrying a device it cannot open, and says why', async () => {
    const lines: string[] = [];
    const h = await PrivateHub.start(HUB, 'serial:/dev/does-not-exist', { log: (l) => lines.push(l) });
    await expect.poll(() => lines.map(deviceProblem).find(Boolean), { timeout: 5000 }).toMatch(/serial: .*\(retrying\)/);
    expect(lines.join('')).not.toContain('\x1b[');
    await h.stop();
  });

  it('reports a hub that cannot start', async () => {
    await expect(PrivateHub.start(HUB, 'bogus:', { timeoutMs: 10_000 })).rejects.toThrow(/bogus/);
  });
});

describe('hub connection states', () => {
  function server(onConn: (s: net.Socket) => void): Promise<{ port: number; close: () => void }> {
    return new Promise((r) => {
      const srv = net.createServer(onConn).listen(0, '127.0.0.1', () => r({ port: (srv.address() as net.AddressInfo).port, close: () => srv.close() }));
    });
  }

  it('busy: closed before any data', async () => {
    const s = await server((c) => c.destroy());
    const states: [LinkState, string?][] = [];
    new HubConnection('127.0.0.1', s.port, (st, d) => states.push([st, d]), () => {});
    await expect.poll(() => states.at(-1)?.[0]).toBe('busy');
    s.close();
  });

  it('approval while silent, then a timeout', async () => {
    const socks: net.Socket[] = [];
    const s = await server((c) => socks.push(c));
    const states: LinkState[] = [];
    new HubConnection('127.0.0.1', s.port, (st) => states.push(st), () => {}, { approvalTimeoutMs: 1500 });
    await expect.poll(() => states, { timeout: 3000 }).toEqual(['connecting', 'approval', 'timeout']);
    socks.forEach((c) => c.destroy());
    s.close();
  });

  it('error when nothing listens', async () => {
    const s = await server(() => {});
    const port = s.port;
    s.close();
    await new Promise((r) => setTimeout(r, 50));
    const states: [LinkState, string?][] = [];
    new HubConnection('127.0.0.1', port, (st, d) => states.push([st, d]), () => {});
    await expect.poll(() => states.at(-1)).toEqual(['error', `no hub at 127.0.0.1:${port}`]);
  });
});

describe('semihost files', () => {
  it('reads, writes, appends and stats inside the root', async () => {
    const root = tmp();
    const fs = new NodeFs(root);
    await fs.mkdir(['logs']);
    const fd = await fs.open(['logs', 'a.txt'], RW);
    expect(await fs.write(fd, new TextEncoder().encode('hello '))).toBe(6);
    await fs.close(fd);
    const ap = await fs.open(['logs', 'a.txt'], { ...RW, read: false, append: true });
    await fs.write(ap, new TextEncoder().encode('world'));
    expect((await fs.stat(ap)).size).toBe(11);
    await fs.close(ap);
    expect(readFileSync(join(root, 'logs/a.txt'), 'utf8')).toBe('hello world');
    const r = await fs.open(['logs', 'a.txt'], RO);
    expect(new TextDecoder().decode(await fs.read(r, 5))).toBe('hello');
    expect(new TextDecoder().decode(await fs.read(r, 100))).toBe(' world');
    expect((await fs.read(r, 10)).length).toBe(0);
    await expect(fs.write(r, new Uint8Array(1))).rejects.toMatchObject({ errno: 'EBADF' });
    await fs.close(r);
    await expect(fs.read(r, 1)).rejects.toMatchObject({ errno: 'EBADF' });
  });

  it('maps errors to newlib names', async () => {
    const root = tmp();
    const fs = new NodeFs(root);
    await expect(fs.open(['missing.txt'], RO)).rejects.toMatchObject({ errno: 'ENOENT' });
    await expect(fs.open(['no-dir', 'x.txt'], RW)).rejects.toMatchObject({ errno: 'ENOENT' });
    await fs.mkdir(['d']);
    await expect(fs.mkdir(['d'])).rejects.toMatchObject({ errno: 'EEXIST' });
    await expect(fs.open(['d'], RO)).rejects.toMatchObject({ errno: 'EISDIR' });
    writeFileSync(join(root, 'f'), 'x');
    await expect(fs.open(['f'], { ...RW, exclusive: true })).rejects.toMatchObject({ errno: 'EEXIST' });
  });

  it('creates a missing root on the first create only', async () => {
    const root = join(tmp(), 'uclog-files');
    const fs = new NodeFs(root);
    await expect(fs.open(['a.txt'], RO)).rejects.toMatchObject({ errno: 'ENOENT' });
    expect(existsSync(root)).toBe(false);
    await fs.close(await fs.open(['a.txt'], RW));
    expect(existsSync(join(root, 'a.txt'))).toBe(true);
  });

  it('refuses paths and symlinks that leave the root', async () => {
    const outside = tmp();
    writeFileSync(join(outside, 'secret'), 'x');
    const root = tmp();
    symlinkSync(join(outside, 'secret'), join(root, 'link'));
    symlinkSync(outside, join(root, 'out'));
    const fs = new NodeFs(root);
    await expect(fs.open(['..', 'x'], RO)).rejects.toMatchObject({ errno: 'EACCES' });
    await expect(fs.open(['a/b'], RO)).rejects.toMatchObject({ errno: 'EACCES' });
    await expect(fs.open(['link'], RO)).rejects.toMatchObject({ errno: 'EACCES' });
    await expect(fs.open(['out', 'new.txt'], RW)).rejects.toMatchObject({ errno: 'EACCES' });
    await expect(fs.mkdir(['out', 'd'])).rejects.toMatchObject({ errno: 'EACCES' });
    for (const p of ['CON', 'nul.txt', 'COM1', 'a:b', 'trail.', 'trail ', 'q?']) {
      await expect(fs.open([p], RW), p).rejects.toMatchObject({ errno: 'EACCES' });
    }
  });

  it('refuses a dangling symlink that O_CREAT would follow out of the root', async () => {
    const outside = tmp();
    const root = tmp();
    symlinkSync(join(outside, 'planted'), join(root, 'dangling'));
    const fs = new NodeFs(root);
    await expect(fs.open(['dangling'], RW)).rejects.toMatchObject({ errno: 'EACCES' });
    await expect(fs.open(['dangling'], { ...RW, truncate: true })).rejects.toMatchObject({ errno: 'EACCES' });
    expect(existsSync(join(outside, 'planted'))).toBe(false);
  });
});

describe('logdata in the extension host', () => {
  const hash = 'ab'.repeat(64);

  it('caches by app_hash', async () => {
    const s = new LogdataStore(join(tmp(), 'logdata'));
    expect(await s.get(hash)).toBeUndefined();
    await s.put(hash, new Uint8Array([1, 2, 3]));
    expect([...(await s.get(hash))!]).toEqual([1, 2, 3]);
    expect(await s.get('../etc/passwd')).toBeUndefined();
    expect(await s.clear()).toBe(1);
    expect(await s.get(hash)).toBeUndefined();
  });

  it('writes the shared cache and still reads the older one', async () => {
    const shared = join(tmp(), 'uclog');
    const older = tmp();
    const other = 'cd'.repeat(64);
    writeFileSync(join(older, `${other}.logdata`), new Uint8Array([9]));
    const s = new LogdataStore(shared, [older]);
    expect([...(await s.get(other))!]).toEqual([9]);
    await s.put(hash, new Uint8Array([1]));
    expect(existsSync(join(shared, `${hash}.logdata`))).toBe(true);
    expect(await s.count()).toBe(1);
    expect(await s.clear()).toBe(2);
    expect(await s.get(other)).toBeUndefined();
    expect(sharedCacheDir({})).toBe(join(homedir(), '.cache', 'uclog'));
    expect(sharedCacheDir({ XDG_CACHE_HOME: '/x' })).toBe(join('/x', 'uclog'));
    expect(sharedCacheDir({ UCLOG_CACHE_DIR: '/c', XDG_CACHE_HOME: '/x' })).toBe('/c');
  });

  it('finds a <hash>.logdata, or the ELF next to a matching signed image', async () => {
    const d = tmp();
    const named = join(d, `${hash}.logdata`);
    writeFileSync(named, 'named');
    const hashes = new ImageHashes();
    expect((await findLocalLogdata(hash, { logdata: [named], images: [] }, hashes))?.name).toBe(named);

    const build = join(d, 'build', 'zephyr');
    mkdirSync(build, { recursive: true });
    const image = new Uint8Array(2048).map((_, i) => i & 0xff);
    writeFileSync(join(build, 'zephyr.signed.bin'), image);
    writeFileSync(join(build, 'zephyr.elf'), 'elf');
    const h = createHash('sha512').update(image.subarray(512)).digest('hex');
    expect(imageHash(image)).toBe(h);
    const r = await findLocalLogdata(h, { logdata: [named], images: [join(build, 'zephyr.signed.bin')] }, hashes);
    expect(r?.name).toBe(join(build, 'zephyr.elf'));
    expect(new TextDecoder().decode(r!.data)).toBe('elf');
    expect(await findLocalLogdata('cd'.repeat(64), { logdata: [named], images: [join(build, 'zephyr.signed.bin')] }, hashes)).toBeUndefined();
  });
});

describe('logdata fetch', () => {
  it('only fetches from the logdata service', () => {
    expect(allowedUrl('https://fw.unitcircle.ca/logdata/acme/aa', 'https://fw.unitcircle.ca')).toBe(true);
    expect(allowedUrl('https://fw.unitcircle.ca/other', 'https://fw.unitcircle.ca')).toBe(false);
    expect(allowedUrl('https://evil.example/logdata/x', 'https://fw.unitcircle.ca')).toBe(false);
    expect(allowedUrl('http://localhost:8080/api/logdata/x', 'http://localhost:8080/api/')).toBe(true);
  });

  it('adds the token and passes status, body and selected headers', async () => {
    let seen: Record<string, string> = {};
    const f = (async (_u: string, init: RequestInit) => {
      seen = init.headers as Record<string, string>;
      return new Response(new Uint8Array([7]), { status: 429, headers: { 'retry-after': '3', 'x-request-id': 'r1', 'set-cookie': 'no' } });
    }) as unknown as typeof fetch;
    const r = await hostFetch('https://fw.unitcircle.ca/logdata/a/b', {}, 'tok', f);
    expect(seen.Authorization).toBe('Bearer tok');
    expect(r.status).toBe(429);
    expect(r.headers).toEqual({ 'retry-after': '3', 'x-request-id': 'r1', 'x-uclog-token-sent': '1' });
    expect([...r.body]).toEqual([7]);
  });

  it('leaves the token out when asked (the retry after a refused token)', async () => {
    let seen: Record<string, string> = {};
    const f = (async (_u: string, init: RequestInit) => {
      seen = init.headers as Record<string, string>;
      return new Response(new Uint8Array([7]));
    }) as unknown as typeof fetch;
    const r = await hostFetch('https://fw.unitcircle.ca/logdata/a/b', { 'x-uclog-no-token': '1' }, 'tok', f);
    expect(seen).toEqual({});
    expect(r.headers['x-uclog-token-sent']).toBeUndefined();
  });
});
