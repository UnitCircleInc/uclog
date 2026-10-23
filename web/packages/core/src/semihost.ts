// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Port-63 semihost requests from the device (protocol.md §5.5). Time ops are answered here
 * (direct links only; a hub answers them itself, D31). File ops go to a pluggable `FsProvider`;
 * without one, OPEN and MKDIR fail with EACCES (§5.5.5).
 */

export const OP = {
  SYSTEM: 0,
  RENAME: 1,
  ISATTY: 2,
  UNLINK: 3,
  LINK: 4,
  STAT: 5,
  FSTAT: 6,
  CLOSE: 7,
  OPEN: 8,
  WRITE: 9,
  LSEEK: 10,
  READ: 11,
  MKDIR: 12,
  TIMEOFDAY: 13,
  SYS_CLOCK: 0x10,
  SYS_TIME: 0x11,
  SYS_ELAPSED: 0x30,
  SYS_TICKFREQ: 0x31,
} as const;

export const OP_NAMES: Record<number, string> = Object.fromEntries(Object.entries(OP).map(([k, v]) => [v, k]));

/** newlib <sys/errno.h> (D25). */
export const ERRNO = {
  EPERM: 1,
  ENOENT: 2,
  EINTR: 4,
  EIO: 5,
  EBADF: 9,
  EAGAIN: 11,
  ENOMEM: 12,
  EACCES: 13,
  EEXIST: 17,
  ENOTDIR: 20,
  EISDIR: 21,
  EINVAL: 22,
  EMFILE: 24,
  ENOSPC: 28,
  ESPIPE: 29,
  EROFS: 30,
  ENOSYS: 88,
  ENOTEMPTY: 90,
  ENAMETOOLONG: 91,
} as const;
export type Errno = keyof typeof ERRNO;

export const O = {
  ACCMODE: 3,
  RDONLY: 0,
  WRONLY: 1,
  RDWR: 2,
  APPEND: 0x8,
  CREAT: 0x200,
  TRUNC: 0x400,
  EXCL: 0x800,
} as const;

export const WRITE_NO_REPLY = 0x80000000;
/** A READ reply fits in one frame (§2.4). */
export const MAX_READ = 1024;
export const MAX_FILES = 64;

const TIME_OPS = new Set<number>([OP.TIMEOFDAY, OP.SYS_CLOCK, OP.SYS_TIME, OP.SYS_ELAPSED, OP.SYS_TICKFREQ]);
export function isTimeOp(op: number): boolean {
  return TIME_OPS.has(op);
}

export function reply(r: number, err = 0, extra = 0): Uint8Array {
  const b = new Uint8Array(8 + extra);
  const dv = new DataView(b.buffer);
  dv.setInt32(0, r, true);
  dv.setInt32(4, err, true);
  return b;
}

/** `<i32 -1, i32 errno>`; FSTAT error replies keep the full 48 bytes (§5.5.4). */
export function errorReply(op: number, name: Errno): Uint8Array {
  return reply(-1, ERRNO[name], op === OP.FSTAT ? 40 : 0);
}

export interface Clock {
  /** Monotonic counter and its ticks per second. */
  ticks(): bigint;
  freq: bigint;
  /** Wall clock, seconds since the epoch. */
  wall(): number;
}

/** §5.5.3 browser source: µs since the time origin. */
export const browserClock: Clock = {
  ticks: () => BigInt(Math.round((performance.timeOrigin + performance.now()) * 1000)),
  freq: 1_000_000n,
  wall: () => Date.now() / 1000,
};

/** Encode a time-op reply (§5.5.3). */
export function timeReply(op: number, ticks: bigint, freq: bigint, wall: number): Uint8Array | undefined {
  const dv = (n: number) => new DataView(new ArrayBuffer(n));
  switch (op) {
    case OP.SYS_CLOCK: {
      const d = dv(4);
      d.setUint32(0, Number(((ticks * 100n) / freq) & 0xffffffffn), true);
      return new Uint8Array(d.buffer);
    }
    case OP.SYS_TIME: {
      const d = dv(8);
      d.setBigInt64(0, BigInt(Math.floor(wall)), true);
      return new Uint8Array(d.buffer);
    }
    case OP.SYS_ELAPSED: {
      const d = dv(8);
      d.setBigUint64(0, ticks & 0xffffffffffffffffn, true);
      return new Uint8Array(d.buffer);
    }
    case OP.SYS_TICKFREQ: {
      const d = dv(8);
      d.setBigUint64(0, freq, true);
      return new Uint8Array(d.buffer);
    }
    case OP.TIMEOFDAY: {
      const sec = Math.floor(wall);
      const usec = Math.min(Math.trunc((wall - sec) * 1e6 + 0.5), 999_999);
      const d = dv(12);
      d.setBigUint64(0, BigInt(sec), true);
      d.setUint32(8, usec, true);
      return new Uint8Array(d.buffer);
    }
  }
  return undefined;
}

export type Fields = Record<string, number | string | boolean | Uint8Array>;

/** Split a request into (op, name, fields) like the reference; throws on a malformed request. */
export function parseRequest(frame: Uint8Array): { op: number; name: string | null; fields: Fields } {
  if (!frame.length) throw new Error('empty request');
  const op = frame[0];
  const a = frame.subarray(1);
  const dv = new DataView(a.buffer, a.byteOffset, a.byteLength);
  const name = OP_NAMES[op] ?? null;
  const f: Fields = {};
  const bad = () => new Error('bad size');
  switch (name) {
    case 'ISATTY':
    case 'FSTAT':
    case 'CLOSE':
      if (a.length !== 4) throw bad();
      f.fid = dv.getInt32(0, true);
      break;
    case 'OPEN':
    case 'MKDIR':
      if (a.length < 4) throw bad();
      f[name === 'OPEN' ? 'flags' : 'mode'] = dv.getInt32(0, true);
      f.path = new TextDecoder('utf-8', { fatal: true }).decode(a.subarray(4));
      break;
    case 'WRITE': {
      if (a.length < 4) throw bad();
      const raw = dv.getUint32(0, true);
      f.fid = raw & 0x7fffffff;
      f.reply = !(raw & WRITE_NO_REPLY);
      f.data = a.slice(4);
      break;
    }
    case 'READ':
      if (a.length !== 8) throw bad();
      f.fid = dv.getInt32(0, true);
      f.len = dv.getUint32(4, true);
      break;
    case 'TIMEOFDAY':
    case 'SYS_CLOCK':
    case 'SYS_TIME':
    case 'SYS_ELAPSED':
    case 'SYS_TICKFREQ':
      if (a.length) throw new Error('time ops take no arguments');
      break;
    default:
      f.args = a.slice();
  }
  return { op, name, fields: f };
}

// ---------------------------------------------------------------------------------------------
// File system provider (the sandbox root; paths arrive already checked and split)
// ---------------------------------------------------------------------------------------------

/** Thrown by providers; `errno` is the newlib name. */
export class FsError extends Error {
  constructor(readonly errno: Errno) {
    super(errno);
  }
}

export interface OpenFlags {
  read: boolean;
  write: boolean;
  append: boolean;
  create: boolean;
  truncate: boolean;
  exclusive: boolean;
}

export interface FsFile {
  read(n: number): Promise<Uint8Array>;
  write(data: Uint8Array): Promise<number>;
  close(): Promise<void>;
  stat(): Promise<{ size: number; mtime: number; dir: boolean }>;
}

export interface FsProvider {
  /** A short description for the UI (folder name). */
  readonly name: string;
  open(path: string[], flags: OpenFlags): Promise<FsFile>;
  mkdir(path: string[]): Promise<void>;
}

/** Split a device path inside the sandbox: absolute paths and escaping `..` → EACCES (§5.5.5). */
export function sandboxPath(path: string): string[] {
  if (!path) throw new FsError('ENOENT');
  if (path.startsWith('/') || path.startsWith('\\') || (path.length > 1 && path[1] === ':')) throw new FsError('EACCES');
  const parts: string[] = [];
  for (const c of path.replace(/\\/g, '/').split('/')) {
    if (c === '' || c === '.') continue;
    if (c === '..') {
      if (!parts.length) throw new FsError('EACCES');
      parts.pop();
    } else parts.push(c);
  }
  if (!parts.length) throw new FsError('EACCES');
  return parts;
}

/** An in-memory provider (tests and the demo). */
export class MemoryFs implements FsProvider {
  readonly files = new Map<string, Uint8Array>();
  readonly dirs = new Set<string>(['']);
  constructor(readonly name = 'memory') {}

  async mkdir(path: string[]) {
    const p = path.join('/');
    if (this.dirs.has(p) || this.files.has(p)) throw new FsError('EEXIST');
    if (!this.dirs.has(path.slice(0, -1).join('/'))) throw new FsError('ENOENT');
    this.dirs.add(p);
  }

  async open(path: string[], f: OpenFlags): Promise<FsFile> {
    const p = path.join('/');
    if (this.dirs.has(p)) throw new FsError('EISDIR');
    if (!this.dirs.has(path.slice(0, -1).join('/'))) throw new FsError('ENOENT');
    if (this.files.has(p)) {
      if (f.create && f.exclusive) throw new FsError('EEXIST');
    } else if (f.create) this.files.set(p, new Uint8Array(0));
    else throw new FsError('ENOENT');
    if (f.truncate && f.write) this.files.set(p, new Uint8Array(0));
    let pos = f.append ? this.files.get(p)!.length : 0;
    const fs = this.files;
    return {
      async read(n) {
        const d = fs.get(p)!;
        const out = d.slice(pos, pos + n);
        pos += out.length;
        return out;
      },
      async write(data) {
        if (!f.write) throw new FsError('EBADF');
        const d = fs.get(p)!;
        if (f.append) pos = d.length;
        const nd = new Uint8Array(Math.max(d.length, pos + data.length));
        nd.set(d);
        nd.set(data, pos);
        fs.set(p, nd);
        pos += data.length;
        return data.length;
      },
      async close() {},
      async stat() {
        return { size: fs.get(p)!.length, mtime: Date.now() / 1000, dir: false };
      },
    };
  }
}

export type ShOutcome =
  | { reply?: Uint8Array; console?: { fid: number; data: Uint8Array } }
  | { stdin: number };

/** Answers port-63 requests for one session. */
export class SemihostHandler {
  private files = new Map<number, FsFile>();
  private nextFid = 3;

  constructor(
    public fs: FsProvider | null = null,
    public clock: Clock = browserClock,
  ) {}

  /** Change the sandbox (open files are closed). */
  async setFs(fs: FsProvider | null) {
    await this.closeAll();
    this.fs = fs;
  }

  async closeAll() {
    for (const f of this.files.values()) await f.close().catch(() => {});
    this.files.clear();
  }

  timeReply(op: number): Uint8Array | undefined {
    return timeReply(op, this.clock.ticks(), this.clock.freq, this.clock.wall());
  }

  async handle(frame: Uint8Array): Promise<ShOutcome> {
    if (!frame.length) return { reply: errorReply(0xff, 'EINVAL') };
    const op = frame[0];
    let req;
    try {
      req = parseRequest(frame);
    } catch {
      return { reply: errorReply(op, 'EINVAL') };
    }
    const f = req.fields;
    try {
      switch (req.name) {
        case 'TIMEOFDAY':
        case 'SYS_CLOCK':
        case 'SYS_TIME':
        case 'SYS_ELAPSED':
        case 'SYS_TICKFREQ':
          return { reply: this.timeReply(op) };
        case 'WRITE': {
          const fid = f.fid as number;
          const data = f.data as Uint8Array;
          let r: Uint8Array;
          let console: { fid: number; data: Uint8Array } | undefined;
          if (fid === 1 || fid === 2) {
            r = reply(data.length);
            console = { fid, data };
          } else {
            const file = this.files.get(fid);
            r = file ? reply(await file.write(data)) : errorReply(op, 'EBADF');
          }
          return { reply: f.reply ? r : undefined, console };
        }
        case 'READ': {
          const fid = f.fid as number;
          const n = Math.min(f.len as number, MAX_READ);
          if (fid === 0) return { stdin: n };
          const file = this.files.get(fid);
          if (!file) return { reply: errorReply(op, 'EBADF') };
          const d = await file.read(n);
          const r = reply(d.length, 0, d.length);
          r.set(d, 8);
          return { reply: r };
        }
        case 'ISATTY': {
          const fid = f.fid as number;
          if (fid >= 0 && fid <= 2) return { reply: reply(1) };
          return { reply: this.files.has(fid) ? reply(0) : errorReply(op, 'EBADF') };
        }
        case 'CLOSE': {
          const fid = f.fid as number;
          if (fid >= 0 && fid <= 2) return { reply: reply(0) };
          const file = this.files.get(fid);
          if (!file) return { reply: errorReply(op, 'EBADF') };
          this.files.delete(fid);
          await file.close();
          return { reply: reply(0) };
        }
        case 'FSTAT': {
          const fid = f.fid as number;
          if (fid >= 0 && fid <= 2) return { reply: fstat(0o020666, 0, 0) };
          const file = this.files.get(fid);
          if (!file) return { reply: errorReply(op, 'EBADF') };
          const s = await file.stat();
          return { reply: fstat((s.dir ? 0o040000 : 0o100000) | 0o644, Math.min(s.size, 0xffffffff), s.mtime) };
        }
        case 'OPEN': {
          if (!this.fs) return { reply: errorReply(op, 'EACCES') };
          const path = sandboxPath(f.path as string);
          if (this.files.size >= MAX_FILES) return { reply: errorReply(op, 'EMFILE') };
          const flags = f.flags as number;
          const acc = flags & O.ACCMODE;
          const file = await this.fs.open(path, {
            read: acc !== O.WRONLY,
            write: acc !== O.RDONLY,
            append: !!(flags & O.APPEND),
            create: !!(flags & O.CREAT),
            truncate: !!(flags & O.TRUNC),
            exclusive: !!(flags & O.EXCL),
          });
          const fid = this.nextFid;
          this.nextFid = this.nextFid >= 0x7ffffff0 ? 3 : this.nextFid + 1;
          this.files.set(fid, file);
          return { reply: reply(fid) };
        }
        case 'MKDIR': {
          if (!this.fs) return { reply: errorReply(op, 'EACCES') };
          await this.fs.mkdir(sandboxPath(f.path as string));
          return { reply: reply(0) };
        }
        default:
          return { reply: errorReply(op, 'ENOSYS') };
      }
    } catch (e) {
      return { reply: errorReply(op, e instanceof FsError ? e.errno : 'EIO') };
    }
  }

  /** READ reply for a stdin line (a trailing CR becomes LF). */
  stdinReply(line: Uint8Array, n: number): Uint8Array {
    let l = line;
    if (l.length && l[l.length - 1] === 13) {
      l = new Uint8Array(l);
      l[l.length - 1] = 10;
    }
    l = l.subarray(0, n);
    const r = reply(l.length, 0, l.length);
    r.set(l, 8);
    return r;
  }
}

function fstat(mode: number, size: number, mtime: number): Uint8Array {
  // <i32 0, i32 0> then mode, ino, dev(0), nlink, uid, gid, size, atime, mtime, ctime (§5.5.2)
  const r = reply(0, 0, 40);
  const dv = new DataView(r.buffer);
  const t = Math.floor(mtime) >>> 0;
  [mode, 0, 0, 1, 0, 0, size, t, t, t].forEach((v, i) => dv.setUint32(8 + i * 4, v >>> 0, true));
  return r;
}
