// Button payloads (the schema shared with the native TUI/GUI), the zip writer, CR LF conversion.

import { expect, test } from 'vitest';
import { decode, type CborValue } from '@uclog/core';
import { buttonArgs, buttonPayload } from '../src/settings.ts';
import { zip } from '../src/util.ts';

test('text buttons send a line with CR', () => {
  expect(new TextDecoder().decode(buttonPayload({ label: 'Help', cmd: 'help' }))).toBe('help\r');
});

test('map buttons: $args become numbers when they parse, text otherwise', () => {
  const b = { label: 'Set', cmd: { cmd: 'set', n: '$n', name: 'dev-$id', f: '$f', plain: 'x', k: 3 } };
  expect(buttonArgs(b)).toEqual([
    ['n', '$n'],
    ['name', '$id'],
    ['f', '$f'],
  ]);
  const v = decode(buttonPayload(b, { 'n $n': '42', 'name $id': '7', 'f $f': '1.5' })) as Map<CborValue, CborValue>;
  expect(Object.fromEntries(v)).toEqual({ cmd: 'set', n: 42, name: 'dev-7', f: 1.5, plain: 'x', k: 3 });
  const w = decode(buttonPayload(b, { 'n $n': 'abc' })) as Map<CborValue, CborValue>;
  expect(w.get('n')).toBe('abc');
});

test('zip: stored entries with a central directory', () => {
  const z = zip([
    { name: 'a.txt', data: 'hello' },
    { name: 'b.csv', data: 'x,y\n' },
  ]);
  const dv = new DataView(z.buffer);
  expect(dv.getUint32(0, true)).toBe(0x04034b50);
  expect(dv.getUint32(z.length - 22, true)).toBe(0x06054b50);
  expect(dv.getUint16(z.length - 22 + 10, true)).toBe(2);
  // CRC-32 of "hello"
  expect(dv.getUint32(14, true)).toBe(0x3610a686);
});
