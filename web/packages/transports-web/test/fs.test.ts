// DirectoryFs against an in-memory folder that behaves like Chrome's: a writable stream's data
// reaches the file only when the stream is closed.

import { describe, expect, test } from 'vitest';
import { FsError, type OpenFlags } from '@uclog/core';
import { DirectoryFs, FLUSH_MS } from '../src/fs.ts';

const err = (name: string) => new DOMException(name, name);

class FakeFile {
  data = new Uint8Array(0);
  lastModified = Date.now();
  failWrites = false;
}

class FakeFileHandle {
  readonly kind = 'file';
  constructor(
    readonly name: string,
    readonly file: FakeFile,
  ) {}

  async getFile() {
    return new File([this.file.data.slice()], this.name, { lastModified: this.file.lastModified });
  }

  async createWritable(o: { keepExistingData?: boolean } = {}) {
    if (this.file.failWrites) throw err('NotAllowedError');
    let swap = o.keepExistingData ? this.file.data.slice() : new Uint8Array(0);
    let pos = 0;
    const file = this.file;
    return {
      async write(x: Uint8Array | { type: 'write'; position: number; data: Uint8Array }) {
        const [at, d] = x instanceof Uint8Array ? [pos, x] : [x.position, x.data];
        const n = new Uint8Array(Math.max(swap.length, at + d.length));
        n.set(swap);
        n.set(d, at);
        swap = n;
        pos = at + d.length;
      },
      async seek(p: number) {
        pos = p;
      },
      async close() {
        file.data = swap;
        file.lastModified = Date.now();
      },
    };
  }
}

class FakeDir {
  readonly kind = 'directory';
  entries = new Map<string, FakeDir | FakeFile>();
  constructor(readonly name: string) {}

  async getDirectoryHandle(name: string, o: { create?: boolean } = {}) {
    const e = this.entries.get(name);
    if (e instanceof FakeDir) return e;
    if (e) throw err('TypeMismatchError');
    if (!o.create) throw err('NotFoundError');
    const d = new FakeDir(name);
    this.entries.set(name, d);
    return d;
  }

  async getFileHandle(name: string, o: { create?: boolean } = {}) {
    let e = this.entries.get(name);
    if (e instanceof FakeDir) throw err('TypeMismatchError');
    if (!e) {
      if (!o.create) throw err('NotFoundError');
      e = new FakeFile();
      this.entries.set(name, e);
    }
    return new FakeFileHandle(name, e);
  }

  text(name: string) {
    return new TextDecoder().decode((this.entries.get(name) as FakeFile).data);
  }
}

const flags = (o: Partial<OpenFlags>): OpenFlags => ({ read: false, write: false, append: false, create: false, truncate: false, exclusive: false, ...o });
const enc = (s: string) => new TextEncoder().encode(s);
const dec = (b: Uint8Array) => new TextDecoder().decode(b);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function setup() {
  const root = new FakeDir('files');
  return { root, fs: new DirectoryFs(root as unknown as FileSystemDirectoryHandle) };
}

describe('DirectoryFs', () => {
  test('writes reach the file before it is closed', async () => {
    const { root, fs } = setup();
    const f = await fs.open(['log.txt'], flags({ write: true, create: true, truncate: true }));
    await f.write(enc('one\n'));
    await f.write(enc('two\n'));
    expect(root.text('log.txt')).toBe('');
    await sleep(FLUSH_MS + 100);
    expect(root.text('log.txt')).toBe('one\ntwo\n'); // the page could close now
    await f.write(enc('three\n'));
    await f.close();
    expect(root.text('log.txt')).toBe('one\ntwo\nthree\n');
  });

  test('handles to a file being written see its writes', async () => {
    const { fs } = setup();
    const w = await fs.open(['a.bin'], flags({ read: true, write: true, create: true }));
    await w.write(enc('hello world'));
    const r = await fs.open(['a.bin'], flags({ read: true }));
    expect(dec(await r.read(5))).toBe('hello');
    expect(dec(await r.read(100))).toBe(' world');
    expect((await r.stat()).size).toBe(11);
    await w.write(enc('!'));
    expect(dec(await r.read(100))).toBe('!');
    await r.close();
    await w.close();
  });

  test('append, keep existing data, truncate', async () => {
    const { root, fs } = setup();
    let f = await fs.open(['t.txt'], flags({ write: true, create: true }));
    await f.write(enc('abcdef'));
    await f.close();
    f = await fs.open(['t.txt'], flags({ write: true })); // keeps the rest of the file
    await f.write(enc('XY'));
    await f.close();
    expect(root.text('t.txt')).toBe('XYcdef');
    const a = await fs.open(['t.txt'], flags({ write: true, append: true }));
    const b = await fs.open(['t.txt'], flags({ write: true, append: true }));
    await a.write(enc('1'));
    await b.write(enc('2'));
    await a.write(enc('3'));
    await a.close();
    await b.close();
    expect(root.text('t.txt')).toBe('XYcdef123');
    f = await fs.open(['t.txt'], flags({ write: true, truncate: true }));
    await f.close();
    expect(root.text('t.txt')).toBe('');
  });

  test('read-only handles read the file itself once nobody writes it', async () => {
    const { root, fs } = setup();
    const f = await fs.open(['r.txt'], flags({ write: true, create: true }));
    await f.write(enc('first'));
    await f.close();
    (root.entries.get('r.txt') as FakeFile).data = enc('changed outside');
    const r = await fs.open(['r.txt'], flags({ read: true }));
    expect(dec(await r.read(100))).toBe('changed outside');
    await expect(r.write(enc('x'))).rejects.toMatchObject({ errno: 'EBADF' });
    await r.close();
    await expect(r.read(1)).rejects.toMatchObject({ errno: 'EBADF' });
  });

  test('errors: missing, exclusive, directories, a write that cannot reach the file', async () => {
    const { root, fs } = setup();
    await expect(fs.open(['none'], flags({ read: true }))).rejects.toMatchObject({ errno: 'ENOENT' });
    await fs.mkdir(['d']);
    await expect(fs.mkdir(['d'])).rejects.toMatchObject({ errno: 'EEXIST' });
    await expect(fs.open(['d'], flags({ read: true }))).rejects.toMatchObject({ errno: 'EISDIR' });
    const f = await fs.open(['d', 'x'], flags({ write: true, create: true, exclusive: true }));
    await expect(fs.open(['d', 'x'], flags({ write: true, create: true, exclusive: true }))).rejects.toMatchObject({ errno: 'EEXIST' });
    await f.write(enc('data'));
    ((root.entries.get('d') as FakeDir).entries.get('x') as FakeFile).failWrites = true;
    const e = await f.close().catch((x: unknown) => x);
    expect(e).toBeInstanceOf(FsError);
    expect((e as FsError).errno).toBe('EACCES');
  });
});
