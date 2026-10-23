// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Semihost file ops in a folder the user granted (File System Access API, plan 02 §6
 * "Serve files from folder…"). The handle is passed to the transport worker.
 */

import { FsError, type FsFile, type FsProvider, type OpenFlags } from '@uclog/core';

function mapError(e: unknown): FsError {
  const n = (e as DOMException)?.name;
  if (n === 'NotFoundError') return new FsError('ENOENT');
  if (n === 'NotAllowedError' || n === 'SecurityError') return new FsError('EACCES');
  if (n === 'TypeMismatchError') return new FsError('EISDIR');
  if (n === 'InvalidModificationError') return new FsError('EEXIST');
  if (n === 'QuotaExceededError') return new FsError('ENOSPC');
  return new FsError('EIO');
}

export class DirectoryFs implements FsProvider {
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
      throw e instanceof FsError ? e : mapError(e);
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
      throw e instanceof FsError ? e : mapError(e);
    }
    let pos = 0;
    let writable: FileSystemWritableFileStream | null = null;
    if (f.write) {
      writable = await fh.createWritable({ keepExistingData: !f.truncate });
      if (f.append) {
        pos = (await fh.getFile()).size;
        await writable.seek(pos);
      }
    }
    return {
      async read(n) {
        if (!f.read) throw new FsError('EBADF');
        const file = await fh.getFile();
        const b = new Uint8Array(await file.slice(pos, pos + n).arrayBuffer());
        pos += b.length;
        return b;
      },
      async write(data) {
        if (!writable) throw new FsError('EBADF');
        await writable.write({ type: 'write', position: pos, data: data as BufferSource });
        pos += data.length;
        return data.length;
      },
      async close() {
        await writable?.close();
        writable = null;
      },
      async stat() {
        const file = await fh.getFile();
        return { size: file.size, mtime: file.lastModified / 1000, dir: false };
      },
    };
  }
}
