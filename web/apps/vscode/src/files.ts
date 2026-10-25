// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Files in the extension host (plan 03 §3, §4): semihost file ops sandboxed to a folder, the logdata
 * cache shared with the other hosts and the device builds (`~/.cache/uclog`), and logdata from
 * local builds (a `<app_hash>.logdata`, or
 * the ELF next to a signed image whose SHA-512 after the 512-byte header is the app_hash).
 * No `vscode` import (unit tested).
 */

import { createHash } from 'node:crypto';
import { constants as C, promises as fsp, type Stats } from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';

/** newlib errno names the device understands (protocol.md §5.5, D25). */
const ERRNO = new Set([
  'EPERM',
  'ENOENT',
  'EIO',
  'EBADF',
  'EAGAIN',
  'ENOMEM',
  'EACCES',
  'EEXIST',
  'ENOTDIR',
  'EISDIR',
  'EINVAL',
  'EMFILE',
  'ENOSPC',
  'EROFS',
  'ENOTEMPTY',
  'ENAMETOOLONG',
]);

export class FsError extends Error {
  constructor(readonly errno: string) {
    super(errno);
  }
}

function mapError(e: unknown): FsError {
  if (e instanceof FsError) return e;
  const code = (e as NodeJS.ErrnoException).code ?? '';
  if (code === 'ELOOP') return new FsError('EACCES');
  if (code === 'ENFILE') return new FsError('EMFILE');
  return new FsError(ERRNO.has(code) ? code : 'EIO');
}

export interface OpenFlags {
  read: boolean;
  write: boolean;
  append: boolean;
  create: boolean;
  truncate: boolean;
  exclusive: boolean;
}

interface Handle {
  fh: fsp.FileHandle;
  pos: number;
  read: boolean;
  write: boolean;
}

const DEVICES = new Set(['CON', 'PRN', 'AUX', 'NUL', 'CONIN$', 'CONOUT$']);

/** The same rule as @uclog/core's portableName (protocol.md §5.5.5), checked again here: no `:`,
 * Windows device names, trailing dot or space, control characters or `<>"|?*`. */
function portableName(s: string): boolean {
  if (!s || /[. ]$/.test(s) || /[\x00-\x1f<>:"|?*]/.test(s)) return false;
  const stem = s.split('.')[0].replace(/ +$/, '').toUpperCase();
  return !DEVICES.has(stem) && !/^(COM|LPT)[0-9\u00b9\u00b2\u00b3]$/.test(stem);
}

/** Semihost files under `root` (the webview has already split and checked the path; checked again
 * here, including symlinks that lead out of the root). */
export class NodeFs {
  private handles = new Map<number, Handle>();
  private nextFd = 1;

  constructor(readonly root: string) {}

  private async resolve(parts: string[], forCreate: boolean): Promise<string> {
    if (!parts.length) throw new FsError('EACCES');
    for (const p of parts) {
      if (!p || p === '.' || p === '..' || /[\\/]/.test(p) || !portableName(p)) throw new FsError('EACCES');
    }
    // The root is created when the device first creates something in it.
    if (forCreate) await fsp.mkdir(this.root, { recursive: true }).catch(() => {});
    const root = await fsp.realpath(this.root).catch(() => {
      throw new FsError('ENOENT');
    });
    const full = path.join(root, ...parts);
    const inside = (p: string) => p === root || p.startsWith(root + path.sep);
    let real: string;
    try {
      real = await fsp.realpath(full);
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code !== 'ENOENT' || !forCreate) throw mapError(e);
      // A new file: its directory must exist and be inside.
      let dir: string;
      try {
        dir = await fsp.realpath(path.dirname(full));
      } catch (e2) {
        throw mapError(e2);
      }
      if (!inside(dir)) throw new FsError('EACCES');
      // realpath also fails on a dangling symlink, which O_CREAT would follow out of the root.
      const name = path.join(dir, path.basename(full));
      if (await fsp.lstat(name).then(() => true, () => false)) throw new FsError('EACCES');
      return name;
    }
    if (!inside(real)) throw new FsError('EACCES');
    return real;
  }

  async open(parts: string[], f: OpenFlags): Promise<number> {
    const p = await this.resolve(parts, f.create);
    // O_NOFOLLOW: `p` is a resolved path, so a symlink there now was put there since (not on Windows).
    let flags = (f.read && f.write ? C.O_RDWR : f.write ? C.O_WRONLY : C.O_RDONLY) | (C.O_NOFOLLOW ?? 0);
    if (f.create) flags |= C.O_CREAT;
    if (f.truncate) flags |= C.O_TRUNC;
    if (f.exclusive) flags |= C.O_EXCL;
    let fh: fsp.FileHandle;
    try {
      const st = await fsp.stat(p).catch(() => null);
      if (st?.isDirectory()) throw new FsError('EISDIR');
      fh = await fsp.open(p, flags, 0o644);
    } catch (e) {
      throw mapError(e);
    }
    const pos = f.append ? (await fh.stat()).size : 0;
    const fd = this.nextFd++;
    this.handles.set(fd, { fh, pos, read: f.read || !f.write, write: f.write });
    return fd;
  }

  private h(fd: number): Handle {
    const h = this.handles.get(fd);
    if (!h) throw new FsError('EBADF');
    return h;
  }

  async read(fd: number, n: number): Promise<Uint8Array> {
    const h = this.h(fd);
    if (!h.read) throw new FsError('EBADF');
    const buf = new Uint8Array(Math.max(0, Math.min(n, 1 << 20)));
    try {
      const { bytesRead } = await h.fh.read(buf, 0, buf.length, h.pos);
      h.pos += bytesRead;
      return buf.subarray(0, bytesRead);
    } catch (e) {
      throw mapError(e);
    }
  }

  async write(fd: number, data: Uint8Array): Promise<number> {
    const h = this.h(fd);
    if (!h.write) throw new FsError('EBADF');
    try {
      const { bytesWritten } = await h.fh.write(data, 0, data.length, h.pos);
      h.pos += bytesWritten;
      return bytesWritten;
    } catch (e) {
      throw mapError(e);
    }
  }

  async close(fd: number): Promise<void> {
    const h = this.h(fd);
    this.handles.delete(fd);
    await h.fh.close().catch(() => {});
  }

  async stat(fd: number): Promise<{ size: number; mtime: number; dir: boolean }> {
    const st = await this.h(fd).fh.stat();
    return { size: st.size, mtime: Math.floor(st.mtimeMs / 1000), dir: st.isDirectory() };
  }

  async mkdir(parts: string[]): Promise<void> {
    const p = await this.resolve(parts, true);
    try {
      await fsp.mkdir(p);
    } catch (e) {
      throw mapError(e);
    }
  }

  /** Close every open file (the device's session ended or the root changed). */
  async closeAll() {
    const all = [...this.handles.values()];
    this.handles.clear();
    await Promise.all(all.map((h) => h.fh.close().catch(() => {})));
  }
}

// ---------------------------------------------------------------------------------------------
// Logdata cache
// ---------------------------------------------------------------------------------------------

const HASH = /^[0-9a-f]{128}$/;

/** The cache shared with `uclog`, the Python library and the device builds: `$UCLOG_CACHE_DIR`,
 * else `$XDG_CACHE_HOME/uclog`, else `~/.cache/uclog`, on every platform. */
export function sharedCacheDir(env: NodeJS.ProcessEnv = process.env): string {
  if (env.UCLOG_CACHE_DIR) return env.UCLOG_CACHE_DIR;
  return path.join(env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache'), 'uclog');
}

/** `<dir>/<app_hash>.logdata`. `older` folders (the extension's cache before it used the shared
 * one) are read after `dir` and cleared with it; new files go to `dir`. */
export class LogdataStore {
  constructor(
    readonly dir: string,
    readonly older: string[] = [],
  ) {}

  async get(hashHex: string): Promise<Uint8Array | undefined> {
    if (!HASH.test(hashHex)) return undefined;
    for (const d of [this.dir, ...this.older]) {
      try {
        return new Uint8Array(await fsp.readFile(path.join(d, `${hashHex}.logdata`)));
      } catch {
        /* next */
      }
    }
    return undefined;
  }

  async put(hashHex: string, data: Uint8Array): Promise<void> {
    if (!HASH.test(hashHex)) throw new Error('bad app_hash');
    await fsp.mkdir(this.dir, { recursive: true });
    const tmp = path.join(this.dir, `.${hashHex}.${process.pid}.tmp`);
    await fsp.writeFile(tmp, data);
    await fsp.rename(tmp, path.join(this.dir, `${hashHex}.logdata`));
  }

  /** The `.logdata` files in `dir`. */
  async count(): Promise<number> {
    return (await fsp.readdir(this.dir).catch(() => [] as string[])).filter((f) => f.endsWith('.logdata')).length;
  }

  async clear(): Promise<number> {
    let n = 0;
    for (const d of [this.dir, ...this.older]) {
      for (const f of await fsp.readdir(d).catch(() => [] as string[])) {
        if (f.endsWith('.logdata') || f.endsWith('.tmp')) {
          await fsp.rm(path.join(d, f), { force: true });
          if (f.endsWith('.logdata')) n++;
        }
      }
    }
    return n;
  }
}

// ---------------------------------------------------------------------------------------------
// Local builds
// ---------------------------------------------------------------------------------------------

/** Size of the signed-image header that app_hash skips. */
export const IMAGE_HEADER = 512;

export function imageHash(data: Uint8Array): string | null {
  return data.length > IMAGE_HEADER ? createHash('sha512').update(data.subarray(IMAGE_HEADER)).digest('hex') : null;
}

/** `zephyr.signed.bin` → `zephyr.elf`, `zephyr.logdata` or `zephyr.elf` in the same folder. */
export async function siblingLogdata(image: string): Promise<string | undefined> {
  const dir = path.dirname(image);
  const stem = path.basename(image).split('.')[0];
  for (const p of [path.join(dir, `${stem}.elf`), path.join(dir, `${stem}.logdata`), path.join(dir, 'zephyr.elf')]) {
    try {
      if ((await fsp.stat(p)).isFile()) return p;
    } catch {
      /* next */
    }
  }
  return undefined;
}

/** Image hashes, remembered while the file is unchanged. */
export class ImageHashes {
  private known = new Map<string, { mtime: number; size: number; hash: string | null }>();

  async get(p: string): Promise<string | null> {
    let st: Stats;
    try {
      st = await fsp.stat(p);
    } catch {
      return null;
    }
    const k = this.known.get(p);
    if (k && k.mtime === st.mtimeMs && k.size === st.size) return k.hash;
    const hash = imageHash(new Uint8Array(await fsp.readFile(p)));
    this.known.set(p, { mtime: st.mtimeMs, size: st.size, hash });
    return hash;
  }
}

/** Logdata for `hashHex` among the workspace's files: a `<hash>.logdata`, else the sibling of a
 * matching image. */
export async function findLocalLogdata(
  hashHex: string,
  files: { logdata: string[]; images: string[] },
  hashes: ImageHashes,
): Promise<{ name: string; data: Uint8Array } | undefined> {
  const h = hashHex.toLowerCase();
  for (const p of files.logdata) {
    if (path.basename(p).split('.')[0].toLowerCase() === h) {
      try {
        return { name: p, data: new Uint8Array(await fsp.readFile(p)) };
      } catch {
        /* gone */
      }
    }
  }
  for (const img of files.images) {
    if ((await hashes.get(img)) !== h) continue;
    const s = await siblingLogdata(img);
    if (s) return { name: s, data: new Uint8Array(await fsp.readFile(s)) };
  }
  return undefined;
}
