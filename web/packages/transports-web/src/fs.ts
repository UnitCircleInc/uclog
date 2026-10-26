// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Semihost file ops in a folder the user granted (File System Access API, plan 02 §6
 * "Serve files from folder…"). The handle is passed to the transport worker.
 *
 * A writable stream only reaches the file when it is closed (until then Chrome keeps the data in a
 * swap file), and its writes are not visible to reads. So a file open for writing is held in
 * memory, shared by all its handles: reads see every write, and the contents are written to the
 * file `FLUSH_MS` after a write and on close. Closing the page loses at most that much.
 */

import { FsError, type FsFile, type FsProvider, type OpenFlags } from '@uclog/core';

/** How long written data may wait in memory before it is written to the file. */
export const FLUSH_MS = 200;

function mapError(e: unknown): FsError {
  if (e instanceof FsError) return e;
  const n = (e as DOMException)?.name;
  if (n === 'NotFoundError') return new FsError('ENOENT');
  if (n === 'NotAllowedError' || n === 'SecurityError') return new FsError('EACCES');
  if (n === 'TypeMismatchError') return new FsError('EISDIR');
  if (n === 'InvalidModificationError') return new FsError('EEXIST');
  if (n === 'QuotaExceededError') return new FsError('ENOSPC');
  return new FsError('EIO');
}

/** A file open for writing: its contents, shared by every handle to it until the last closes. */
class OpenFile {
  data = new Uint8Array(0); // capacity; the contents are data[0, size)
  size = 0;
  mtime = Date.now() / 1000;
  handles = 0;
  ready: Promise<void>;
  private dirty = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private flushing: Promise<void> = Promise.resolve();
  private error: FsError | null = null;

  constructor(
    readonly fh: FileSystemFileHandle,
    truncate: boolean,
  ) {
    this.ready = truncate
      ? Promise.resolve().then(() => this.changed()) // the file is emptied soon, as by O_TRUNC
      : fh.getFile().then(async (f) => {
          this.data = new Uint8Array(await f.arrayBuffer());
          this.size = this.data.length;
          this.mtime = f.lastModified / 1000;
        });
  }

  read(pos: number, n: number): Uint8Array {
    return this.data.slice(Math.min(pos, this.size), Math.min(pos + n, this.size));
  }

  write(pos: number, b: Uint8Array) {
    if (this.error) throw this.error;
    const end = pos + b.length;
    if (end > this.data.length) {
      const d = new Uint8Array(Math.max(end, this.data.length * 2, 4096));
      d.set(this.data.subarray(0, this.size));
      this.data = d;
    }
    // A write past the end leaves zeros in between, as on POSIX.
    if (pos > this.size) this.data.fill(0, this.size, pos);
    this.data.set(b, pos);
    this.size = Math.max(this.size, end);
    this.changed();
  }

  truncate() {
    this.size = 0;
    this.changed();
  }

  private changed() {
    this.mtime = Date.now() / 1000;
    this.dirty = true;
    this.timer ??= setTimeout(() => {
      this.timer = undefined;
      void this.flush();
    }, FLUSH_MS);
  }

  /** Write the contents to the file (one at a time; a write that fails is reported once). */
  async flush(): Promise<void> {
    clearTimeout(this.timer);
    this.timer = undefined;
    if (this.dirty) {
      this.dirty = false;
      const snapshot = this.data.slice(0, this.size);
      this.flushing = this.flushing.then(async () => {
        try {
          const w = await this.fh.createWritable({ keepExistingData: false });
          await w.write(snapshot);
          await w.close();
        } catch (e) {
          this.error ??= mapError(e);
        }
      });
    }
    await this.flushing;
    if (this.error) {
      const e = this.error;
      this.error = null;
      throw e;
    }
  }
}

export class DirectoryFs implements FsProvider {
  private files = new Map<string, OpenFile>();

  constructor(readonly root: FileSystemDirectoryHandle) {}

  get name() {
    return this.root.name;
  }

  private async dir(parts: string[], create = false): Promise<FileSystemDirectoryHandle> {
    let d = this.root;
    for (const p of parts) d = await d.getDirectoryHandle(p, { create });
    return d;
  }

  async mkdir(path: string[]) {
    try {
      const parent = await this.dir(path.slice(0, -1));
      const name = path[path.length - 1];
      try {
        await parent.getDirectoryHandle(name);
        throw new FsError('EEXIST');
      } catch (e) {
        if (e instanceof FsError) throw e;
      }
      await parent.getDirectoryHandle(name, { create: true });
    } catch (e) {
      throw mapError(e);
    }
  }

  async open(path: string[], f: OpenFlags): Promise<FsFile> {
    let fh: FileSystemFileHandle;
    try {
      const parent = await this.dir(path.slice(0, -1));
      const name = path[path.length - 1];
      let exists = true;
      try {
        fh = await parent.getFileHandle(name);
      } catch (e) {
        if ((e as DOMException).name === 'TypeMismatchError') throw new FsError('EISDIR');
        if (!f.create) throw new FsError('ENOENT');
        exists = false;
        fh = await parent.getFileHandle(name, { create: true });
      }
      if (exists && f.create && f.exclusive) throw new FsError('EEXIST');
    } catch (e) {
      throw mapError(e);
    }

    // Handles to a file open for writing share its contents; a read-only handle to a file nobody
    // writes reads the file itself.
    const key = path.join('/');
    let file = this.files.get(key);
    if (f.write && !file) {
      file = new OpenFile(fh, f.truncate);
      this.files.set(key, file);
    } else if (file && f.write && f.truncate) {
      await file.ready;
      file.truncate();
    }
    if (file) {
      file.handles++;
      try {
        await file.ready;
      } catch (e) {
        this.release(key, file);
        throw mapError(e);
      }
    }
    const shared = file;
    let pos = f.append && shared ? shared.size : 0;
    let closed = false;
    const live = () => {
      if (closed) throw new FsError('EBADF');
    };
    return {
      read: async (n) => {
        live();
        if (!f.read) throw new FsError('EBADF');
        let b: Uint8Array;
        if (shared) {
          b = shared.read(pos, n);
        } else {
          const file = await fh.getFile().catch((e) => {
            throw mapError(e);
          });
          b = new Uint8Array(await file.slice(pos, pos + n).arrayBuffer());
        }
        pos += b.length;
        return b;
      },
      write: async (data) => {
        live();
        if (!f.write || !shared) throw new FsError('EBADF');
        if (f.append) pos = shared.size; // every write goes to the end (O_APPEND)
        shared.write(pos, data);
        pos += data.length;
        return data.length;
      },
      close: async () => {
        if (closed) return;
        closed = true;
        if (!shared) return;
        try {
          await shared.flush();
        } finally {
          this.release(key, shared);
        }
      },
      stat: async () => {
        live();
        if (shared) return { size: shared.size, mtime: shared.mtime, dir: false };
        const file = await fh.getFile().catch((e) => {
          throw mapError(e);
        });
        return { size: file.size, mtime: file.lastModified / 1000, dir: false };
      },
    };
  }

  private release(key: string, file: OpenFile) {
    if (--file.handles === 0 && this.files.get(key) === file) this.files.delete(key);
  }
}
