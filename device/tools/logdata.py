#!/usr/bin/env python3
# © 2025 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
"""Build the .logdata CBOR for an ELF file (spec/protocol.md §6.1). Python standard library only.

Derived from scripts/logdata.py in github.com/UnitCircleInc/uclog:
- format strings come from the ELF's `.logdata` section (start symbol `_slogstr`);
- enums, typedef'd enums, functions and variables come from `<objdump> -Wi` (DWARF).

Usage: logdata.py [--objdump arm-none-eabi-objdump] [--prefix-map OLD=NEW]... [--dump] ELF -o OUT.logdata
       (--ofile: as -o)

Variables are named <file>:<name>, the file from the DWARF compile unit. Files are written with /,
mapped by --prefix-map as the compiler's -ffile-prefix-map (the last that matches wins), and an
absolute path left after that becomes its file name alone: no build machine's paths (or Windows
drive letters, whose ':' the hosts would split on) in the logdata.
"""

import argparse
import posixpath
import re
import struct
import subprocess
import sys

# ---------------------------------------------------------------------------------------------
# ELF (32-bit little endian is all we need, but handle 64-bit too)


class Elf:
  def __init__(self, data: bytes):
    if data[:4] != b'\x7fELF':
      raise ValueError('not an ELF file')
    self.data = data
    is64 = data[4] == 2
    end = '<' if data[5] == 1 else '>'
    if is64:
      shoff, = struct.unpack_from(end + 'Q', data, 0x28)
      shentsize, shnum, shstrndx = struct.unpack_from(end + 'HHH', data, 0x3A)
      shfmt = end + 'IIQQQQIIQQ'
    else:
      shoff, = struct.unpack_from(end + 'I', data, 0x20)
      shentsize, shnum, shstrndx = struct.unpack_from(end + 'HHH', data, 0x2E)
      shfmt = end + 'IIIIIIIIII'
    self.end, self.is64 = end, is64
    raw = [struct.unpack_from(shfmt, data, shoff + i * shentsize) for i in range(shnum)]
    names = raw[shstrndx]
    self.sections = {}
    for s in raw:
      name, type_, flags, addr, off, size, link, info, align, entsize = s
      self.sections[self._str(names[4], name)] = dict(type=type_, addr=addr, off=off, size=size,
                                                      link=link, entsize=entsize)
    self._raw = raw

  def _str(self, off, i):
    e = self.data.index(b'\0', off + i)
    return self.data[off + i:e].decode()

  def section(self, name):
    s = self.sections.get(name)
    if s is None:
      return None
    if s['type'] == 8:  # NOBITS
      return b''
    return self.data[s['off']:s['off'] + s['size']]

  def symbol(self, name):
    sym = self.sections.get('.symtab')
    if sym is None:
      return None
    strtab = self._raw[sym['link']][4]
    fmt = self.end + ('IBBHQQ' if self.is64 else 'IIIBBH')
    for i in range(sym['size'] // sym['entsize']):
      f = struct.unpack_from(fmt, self.data, sym['off'] + i * sym['entsize'])
      n, value = (f[0], f[4]) if self.is64 else (f[0], f[1])
      if self._str(strtab, n) == name:
        return value
    return None


# ---------------------------------------------------------------------------------------------
# DWARF via objdump -Wi (as the original)

re_header = re.compile(r'^\s*<(\d+)><([0-9a-fA-F]+)>: Abbrev Number: (\d+) \(DW_TAG_(.+)\)')
re_detail = re.compile(r'^\s*<([0-9a-fA-F]+)>\s*DW_AT_(\S+)\s*:(.*)$')
re_addr = re.compile(r'\(DW_OP_addr:\s*([0-9a-fA-F]+)(;.*)?\)')


class Attr:
  def __init__(self, detail):
    self.detail = detail

  @property
  def short(self):
    return self.detail.split(':')[-1].strip()

  @property
  def value(self):
    z = self.detail.strip().split()[0]
    return int(z, 0)

  @property
  def address(self):
    m = re_addr.search(self.detail)
    return int(m.group(1), 16) if m else None

  @property
  def type(self):
    return self.detail.strip()[3:-1]


class Item:
  def __init__(self, tag):
    self.tag = tag
    self.attr = {}
    self.children = []

  def __getitem__(self, k):
    return self.attr[k]

  def __contains__(self, k):
    return k in self.attr

  def type(self, items):
    return items.get(self['type'].type)


def parse_dwarf(objdump, filename):
  items = {}
  stack = [(-1, Item('root'))]
  out = subprocess.run([objdump, '-Wi', filename], check=True, capture_output=True).stdout
  for line in out.decode('utf-8', 'replace').splitlines():
    m = re_detail.match(line)
    if m:
      stack[-1][1].attr[m.group(2)] = Attr(m.group(3))
      continue
    m = re_header.match(line)
    if m:
      level = int(m.group(1))
      item = Item(m.group(4))
      items[m.group(2)] = item
      while stack[-1][0] >= level:
        _, x = stack.pop()
        stack[-1][1].children.append(x)
      stack.append((level, item))
  while len(stack) > 1:
    _, x = stack.pop()
    stack[-1][1].children.append(x)
  return stack[0][1], items


def enum_map(e):
  return {c['const_value'].value: c['name'].short for c in e.children if 'const_value' in c}


def unix_path(p):
  p = posixpath.normpath(p.replace('\\', '/'))
  return p if p != '.' else ''


def source_name(path, prefix_maps=()):
  """A compile unit's file as the logdata names it: see the module docstring."""
  p = unix_path(path)
  for old, new in reversed(prefix_maps):
    old = unix_path(old)
    if old and (p == old or p.startswith(old.rstrip('/') + '/')):
      rest = p[len(old):].lstrip('/')
      new = unix_path(new).rstrip('/')
      p = f'{new}/{rest}' if new and rest else new or rest
      break
  if p.startswith('/') or re.match(r'[A-Za-z]:', p):
    p = posixpath.basename(p)
  return p


def extract(root, items, prefix_maps=()):
  enums, tdenums, variables, functions = {}, {}, {}, {}
  for cu in root.children:
    cu_name = source_name(cu['name'].short, prefix_maps) if 'name' in cu else '?'
    for item in cu.children:
      if item.tag == 'subprogram':
        if 'low_pc' in item and 'name' in item and 'high_pc' in item:
          low = item['low_pc'].value
          if low > 0:
            functions[(low, item['high_pc'].value)] = item['name'].short
      elif item.tag == 'variable':
        if 'location' in item and 'name' in item:
          addr = item['location'].address
          if addr:
            variables[addr] = cu_name + ':' + item['name'].short
      elif item.tag == 'enumeration_type':
        if 'name' in item and item['name'].short not in enums:
          enums[item['name'].short] = enum_map(item)
      elif item.tag == 'typedef':
        if 'type' not in item or 'name' not in item:
          continue
        name = item['name'].short
        c = item.type(items)
        while c is not None and c.tag == 'typedef' and 'type' in c:
          c = c.type(items)
        if c is not None and c.tag == 'enumeration_type' and name not in tdenums:
          m = enum_map(c)
          if m:
            tdenums[name] = m
  return enums, tdenums, variables, functions


# ---------------------------------------------------------------------------------------------
# Format strings

# %[flags][width][.prec][length]conv, optionally preceded by {sym} or {enum:name}.
re_fmt = re.compile(
    r'((?<!%)|(?P<sym>\{sym\})|(\{enum:(?P<enum>[^}]*)\}))'
    r'%(?P<spec>[#+\- 0]*[0-9]*(?:\.[0-9]*)?)(?P<len>hh|h|ll|l|L|z|j|t)?(?P<conv>[diouxXcfFeEgGaAps])')

INT_CONV = {'d': 'int32', 'i': 'int32', 'o': 'int32', 'u': 'uint32', 'x': 'uint32', 'X': 'uint32',
            'c': 'uint32'}


def cfmt2pfmt(fmt):
  """C format → (format with sym/enum/%p/length rewritten, argument kinds)."""
  kinds = []
  out = ''
  while True:
    m = re_fmt.search(fmt)
    if not m:
      return out + fmt, kinds
    spec, ln, conv = m.group('spec'), m.group('len') or '', m.group('conv')
    if m.group('sym'):
      kinds.append('sym')
      infix = '%s'
    elif m.group('enum') is not None:
      kinds.append(['enum', m.group('enum').strip()])
      infix = '%s'
    else:
      if conv in INT_CONV:
        wide = ln in ('ll', 'j')
        k = INT_CONV[conv]
        kinds.append(k.replace('32', '64') if wide else k)
      elif conv == 'p':
        kinds.append('pointer')
      elif conv == 's':
        kinds.append('string')
      else:
        kinds.append('double')
      if conv == 'p':
        infix = '0x%08x' if spec == '' else '%' + spec + 'x'
      else:
        # Only 'l' survives (ll -> l), as the original generator did.
        infix = '%' + spec + ('l' if ln in ('l', 'll') else '') + conv
    out += fmt[:m.start()] + infix
    fmt = fmt[m.end():]


def parse_prefix(prefix):
  if prefix.count(':') >= 3:
    level, fname, line, fmt = prefix.split(':', 3)
    clean, kinds = cfmt2pfmt(fmt)
    return [level, fname, line, clean, kinds]
  if prefix.count(':') == 2:
    return prefix.split(':', 2)
  return None


def load_fmts(elf):
  data = elf.section('.logdata')
  saddr = elf.symbol('_slogstr')
  if not data or saddr is None:
    return None, {}
  fmts = {}
  for m in re.finditer(rb'([^\x00]+)\x00+', data):
    entry = parse_prefix(m.group(1).decode('utf-8', 'replace'))
    if entry is not None:
      fmts[m.start() + saddr] = entry
  return saddr, fmts


# ---------------------------------------------------------------------------------------------
# CBOR


def _head(major, n):
  if n < 24:
    return bytes(((major << 5) | n,))
  for ai, f in ((24, '>B'), (25, '>H'), (26, '>I'), (27, '>Q')):
    if n < (1 << (8 * struct.calcsize(f))):
      return bytes(((major << 5) | ai,)) + struct.pack(f, n)
  raise ValueError('integer too large')


def cbor(v):
  if isinstance(v, bool) or v is None:
    return {None: b'\xf6', True: b'\xf5', False: b'\xf4'}[v]
  if isinstance(v, int):
    return _head(0, v) if v >= 0 else _head(1, -1 - v)
  if isinstance(v, str):
    b = v.encode()
    return _head(3, len(b)) + b
  if isinstance(v, (list, tuple)):
    return _head(4, len(v)) + b''.join(cbor(x) for x in v)
  if isinstance(v, dict):
    return _head(5, len(v)) + b''.join(cbor(k) + cbor(x) for k, x in v.items())
  raise TypeError(type(v))


def main():
  ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
  ap.add_argument('elf')
  ap.add_argument('-o', '--output', '--ofile')
  ap.add_argument('--objdump', default='arm-none-eabi-objdump')
  ap.add_argument('--prefix-map', action='append', default=[], metavar='OLD=NEW',
                  help="map source paths as the compiler's -ffile-prefix-map (repeatable)")
  ap.add_argument('--dump', action='store_true', help='print the format table')
  a = ap.parse_args()
  maps = [tuple(m.split('=', 1)) for m in a.prefix_map]
  if any(len(m) != 2 for m in maps):
    ap.error('--prefix-map takes OLD=NEW')

  with open(a.elf, 'rb') as f:
    elf = Elf(f.read())
  saddr, fmts = load_fmts(elf)
  if saddr is None:
    sys.exit(f'{a.elf}: no .logdata section or _slogstr symbol')
  enums, tdenums, variables, functions = extract(*parse_dwarf(a.objdump, a.elf), maps)

  if a.dump:
    for addr in sorted(fmts):
      print(f'0x{addr:08x}', fmts[addr])
    for name, m in sorted({**enums, **tdenums}.items()):
      print(name, m)

  doc = {
      'enums': {k: enums[k] for k in sorted(enums)},
      'tdenums': {k: tdenums[k] for k in sorted(tdenums)},
      'vars': {k: variables[k] for k in sorted(variables)},
      'fns': {k: functions[k] for k in sorted(functions)},
      'saddr': saddr,
      'fmts': {k: fmts[k] for k in sorted(fmts)},
  }
  if a.output:
    with open(a.output, 'wb') as f:
      f.write(cbor(doc))
  print(f'{a.output or a.elf}: {len(fmts)} formats, {len(enums)} enums, {len(tdenums)} typedef enums, '
        f'{len(functions)} functions, {len(variables)} variables, target {(saddr >> 20) & 0xF}')


if __name__ == '__main__':
  main()
