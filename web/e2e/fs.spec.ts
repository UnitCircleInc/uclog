// The semihost folder provider (DirectoryFs) against Chrome's File System Access implementation,
// in the origin-private file system (the same handles as a folder the user picks, without the
// picker): data written reaches the file before the device closes it, and other handles see it.

import { join } from 'node:path';
import { expect, test } from '@playwright/test';

const FS_TS = join(import.meta.dirname, '../packages/transports-web/src/fs.ts');

test('DirectoryFs in Chrome: writes reach the file before close', async ({ page }) => {
  await page.goto('/');
  const r = await page.evaluate(async (src) => {
    const { DirectoryFs, FLUSH_MS } = await import(/* @vite-ignore */ `/@fs${src}`);
    const root = await navigator.storage.getDirectory();
    const dir = await root.getDirectoryHandle(`uclog-e2e-${Date.now()}`, { create: true });
    const fs = new DirectoryFs(dir);
    const fl = { read: true, write: true, append: false, create: true, truncate: true, exclusive: false };
    const enc = (s: string) => new TextEncoder().encode(s);
    const onDisk = async () => (await (await dir.getFileHandle('t.txt')).getFile()).text();
    const w = await fs.open(['t.txt'], fl);
    await w.write(enc('hello '));
    await w.write(enc('world'));
    const before = await onDisk();
    await new Promise((res) => setTimeout(res, FLUSH_MS + 300));
    const flushed = await onDisk();
    const reader = await fs.open(['t.txt'], { ...fl, write: false, create: false, truncate: false });
    const seen = new TextDecoder().decode(await reader.read(100));
    await w.write(enc('!'));
    await w.close();
    await reader.close();
    const closed = await onDisk();
    await root.removeEntry(dir.name, { recursive: true });
    return { before, flushed, seen, closed };
  }, FS_TS);
  expect(r).toEqual({ before: '', flushed: 'hello world', seen: 'hello world', closed: 'hello world!' });
});
