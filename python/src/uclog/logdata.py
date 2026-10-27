# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Logdata (.logdata CBOR, or the .logdata_cbor section of an ELF file) and log decoding
(protocol.md §6). Error texts match the native tools (plans README §5.6)."""

import os
import re
import struct
from dataclasses import dataclass
from typing import Optional

from . import cbor
from .proto import TARGET_DIGIT_SHIFT, TYPE_MEM

__all__ = ['LogData', 'LogRecord', 'DecodeError', 'LogDataError', 'LEVELS']

LEVELS = ('DEBUG', 'INFO', 'WARN', 'ERROR', 'FATAL', 'PANIC')


class LogDataError(ValueError):
  """A logdata file could not be loaded."""


class DecodeError(ValueError):
  """A log frame could not be decoded. `kind` is the error-record kind (`unknown_format`,
  `missing_arg`, `extra_bytes`, `format`)."""

  def __init__(self, kind, text):
    super().__init__(text)
    self.kind = kind


@dataclass(frozen=True)
class LogRecord:
  """A decoded log line, or an error record when `error` is set (protocol.md §7)."""
  ts: float
  level: str
  target: Optional[int]
  file: str
  line: str
  text: str
  error: Optional[str] = None

  def __str__(self):
    # The legacy `uclog-cli.py` line format.
    return f'{self.ts:.3f}:{self.level}:{self.file}:{self.line}:{self.text}'


# ---------------------------------------------------------------------------
# ELF: just enough to find one section by name
# ---------------------------------------------------------------------------

def elf_section(data: bytes, name: str) -> Optional[bytes]:
  """Contents of section `name` of an ELF32/ELF64 file (either byte order), or None."""
  if data[:4] != b'\x7fELF' or len(data) < 0x34:
    raise LogDataError('not an ELF file')
  is64 = data[4] == 2
  e = '<' if data[5] == 1 else '>'
  if is64:
    shoff, = struct.unpack_from(e + 'Q', data, 0x28)
    shentsize, shnum, shstrndx = struct.unpack_from(e + 'HHH', data, 0x3A)
  else:
    shoff, = struct.unpack_from(e + 'I', data, 0x20)
    shentsize, shnum, shstrndx = struct.unpack_from(e + 'HHH', data, 0x2E)

  def section(i):
    o = shoff + i * shentsize
    if is64:
      nm, _t, _f, _a, off, size = struct.unpack_from(e + 'IIQQQQ', data, o)
    else:
      nm, _t, _f, _a, off, size = struct.unpack_from(e + 'IIIIII', data, o)
    return nm, off, size

  try:
    _, stroff, strsize = section(shstrndx)
    names = data[stroff:stroff + strsize]
    want = name.encode()
    for i in range(shnum):
      nm, off, size = section(i)
      end = names.find(b'\0', nm)
      if names[nm:end] == want:
        return bytes(data[off:off + size])
  except struct.error as ex:
    raise LogDataError(f'truncated ELF file: {ex}') from None
  return None


# ---------------------------------------------------------------------------
# C printf, as the native tools format it
# ---------------------------------------------------------------------------

_SPEC = re.compile(r'%(?P<flags>[-+ #0]*)(?P<width>[0-9]*)(?:\.(?P<prec>[0-9]*))?'
                   r'(?:hh|ll|h|l|z|j|t|L)?(?P<conv>[diouxXcfFeEgGaAs%])')


class _Int(int):
  """An integer argument and its size in bytes (for %u/%x of negative values)."""

  def __new__(cls, v, size):
    o = super().__new__(cls, v)
    o.size = size
    return o


def _as_int(v):
  if isinstance(v, float):
    return int(v)
  if isinstance(v, str):
    try:
      return int(v)
    except ValueError:
      return 0
  return int(v)


def _as_float(v):
  if isinstance(v, str):
    try:
      return float(v)
    except ValueError:
      return float('nan')
  return float(v)


def _convert(m, v):
  flags, width, prec, conv = m.group('flags'), m.group('width'), m.group('prec'), m.group('conv')
  spec = '%' + flags.replace('#', '') + width + ('' if prec is None else '.' + (prec or '0'))
  if conv in 'di':
    if prec is not None:  # C ignores the 0 flag when there is a precision
      spec = '%' + flags.replace('#', '').replace('0', '') + width + '.' + (prec or '0')
    return (spec + 'd') % _as_int(v)
  if conv in 'ouxX':
    u = _as_int(v)
    if u < 0:
      u &= (1 << (8 * getattr(v, 'size', 8))) - 1
    spec = spec.replace('+', '').replace(' ', '')
    if conv == 'u':
      return (spec + 'd') % u
    alt = '#' in flags and u != 0
    if conv == 'o':
      s = '%o' % u
      if prec is not None:
        s = s.rjust(int(prec or 0), '0')
      s = ('0' + s) if alt and not s.startswith('0') else s
      return _pad(flags, width, '', s)
    digits = ('%x' if conv == 'x' else '%X') % u
    if prec is not None:
      digits = digits.rjust(int(prec or 0), '0')
    return _pad(flags, width, ('0' + conv) if alt else '', digits, numeric=prec is None)
  if conv == 'c':
    ch = v[:1] if isinstance(v, str) else chr(_as_int(v) & 0x10FFFF)
    return _pad(flags, width, '', ch)
  if conv == 's':
    t = v if isinstance(v, str) else str(v)
    if prec is not None:
      t = t[:int(prec or 0)]
    return _pad(flags, width, '', t)
  f = _as_float(v)
  if conv in 'aA':
    s = _hex_float(f)
    return _pad(flags, width, '', s.upper() if conv == 'A' else s)
  return ((spec.replace('%', '%#', 1) if '#' in flags else spec) + conv) % f


def _hex_float(f):
  """C %a: 0x1.<hex digits>p<exp> with trailing zero digits removed (0x1p+0, 0x0p+0)."""
  if f != f or f in (float('inf'), float('-inf')):
    return str(f)
  mant, exp = f.hex().split('p')  # Python always gives 13 hex digits
  if '.' in mant:
    mant = mant.rstrip('0').rstrip('.')
  return f'{mant}p{exp}'


def _pad(flags, width, prefix, body, numeric=False):
  w = int(width or 0)
  n = w - len(prefix) - len(body)
  if n <= 0:
    return prefix + body
  if '-' in flags:
    return prefix + body + ' ' * n
  if '0' in flags and numeric:
    return prefix + '0' * n + body
  return ' ' * n + prefix + body


def format_c(fmt: str, values) -> str:
  """Format with C printf semantics. Raises ValueError when the number of conversions and
  values differ."""
  out = []
  pos = 0
  it = iter(values)
  for m in _SPEC.finditer(fmt):
    out.append(fmt[pos:m.start()])
    pos = m.end()
    if m.group('conv') == '%':
      out.append('%')
      continue
    try:
      v = next(it)
    except StopIteration:
      raise ValueError('format has more conversions than arguments') from None
    out.append(_convert(m, v))
  out.append(fmt[pos:])
  if next(it, None) is not None:
    raise ValueError('format has fewer conversions than arguments')
  return ''.join(out)


_ESC = {'\\': '\\\\', '\n': '\\n', '\r': '\\r', '\t': '\\t'}


def escape_str(v: str) -> str:
  """Escape a %s argument (§6.3)."""
  out = []
  for ch in v:
    if ch in _ESC:
      out.append(_ESC[ch])
    elif ord(ch) < 0x20 or ord(ch) == 0x7F:
      out.append(f'\\x{ord(ch):02x}')
    else:
      out.append(ch)
  return ''.join(out)


# ---------------------------------------------------------------------------
# LogData
# ---------------------------------------------------------------------------

_INT_KINDS = {'int32': ('<i', 4), 'uint32': ('<I', 4), 'pointer': ('<I', 4),
              'int64': ('<q', 8), 'uint64': ('<Q', 8)}


class LogData:
  """One logdata file: the formats of one target (§6.1)."""

  def __init__(self, data: bytes, source: str = '<bytes>'):
    try:
      a = cbor.loads(data)
      self.enums = a['enums']
      self.tdenums = a['tdenums']
      self.vars = dict(sorted(a['vars'].items()))
      self.fns = sorted((low, size, name) for (low, size), name in a['fns'].items())
      self.saddr = a['saddr']
      self.fmts = a['fmts']
    except (cbor.CborError, KeyError, TypeError, ValueError) as e:
      raise LogDataError(f'{source}: not a logdata file ({e})') from None
    self.source = source
    #: The app_hash from a `<hash>.logdata` file name, if any.
    self.app_hash: Optional[bytes] = None

  @classmethod
  def load(cls, path) -> 'LogData':
    """Load a `.logdata` file or an ELF file with a `.logdata_cbor` section."""
    path = os.fspath(path)
    with open(path, 'rb') as f:
      data = f.read()
    if data[:4] == b'\x7fELF':
      sec = elf_section(data, '.logdata_cbor')
      if not sec:
        raise LogDataError(f'{path}: no .logdata_cbor section in ELF file')
      data = sec
    ld = cls(data, path)
    stem = os.path.basename(path).split('.')[0]
    if len(stem) == 128:
      try:
        ld.app_hash = bytes.fromhex(stem)
      except ValueError:
        pass
    return ld

  @property
  def target(self) -> int:
    """The target digit this logdata decodes."""
    return (self.saddr >> TARGET_DIGIT_SHIFT) & 0xF

  def lookup_sym(self, a: int) -> str:
    """An address as `{sym}%p` shows it: `function+0xOFF`, else the nearest variable below it
    (within 0x3000) as `file:name+0xOFF`, else hex (protocol.md §6.3)."""
    a2 = a & ~1
    for low, size, name in self.fns:
      if low <= a2 < low + size:
        return f'{name}+0x{a2 - low:x}'
    best = None
    for addr, name in self.vars.items():
      if addr <= a:
        best = (addr, name)
      else:
        break
    if best and a - best[0] < 0x3000:
      return f'{best[1]}+0x{a - best[0]:x}'
    return f'0x{a:08x}'

  def enum_name(self, name: str, v: int) -> str:
    """An enum value as `{enum:name}%d` shows it: the enumerator, `<name:v>` for a value the enum
    lacks, or `<!name:v>` when there is no such enum."""
    for t in (self.enums, self.tdenums):
      if name in t:
        return t[name].get(v, f'<{name}:{v}>')
    return f'<!{name}:{v}>'

  def decode(self, addr: int, args: bytes):
    """Decode one type-0/1 log frame. Returns (level, file, line, text); raises DecodeError
    (and nothing else, even for a malformed logdata entry)."""
    try:
      return self._decode(addr, args)
    except DecodeError:
      raise
    except (IndexError, KeyError, TypeError, AttributeError, ValueError, struct.error) as e:
      raise DecodeError('format', f'malformed logdata entry for addr 0x{addr & ~3:08x} '
                        f'in {self.source}: {type(e).__name__}: {e}') from None

  def _decode(self, addr, args):
    key = addr & ~3
    entry = self.fmts.get(key)
    if not isinstance(entry, (list, tuple)) or len(entry) != 5:
      raise DecodeError('unknown_format', f'unknown format addr 0x{key:08x}')
    level, fname, line, fmt, kinds = entry
    level = LEVELS[int(level)] if str(level).isdigit() and int(level) < len(LEVELS) else f'<bad level {level}>'
    where = f'{fname}:{line} "{fmt}"'

    def missing(i, kind):
      return DecodeError('missing_arg', f'{where} — missing arg {i} ({kind})')

    if addr & 3 == TYPE_MEM:
      if len(args) < 4:
        raise missing(0, 'uint32')
      ptr, = struct.unpack_from('<I', args)
      rest = args[4:]
      if line == '<zephyr>':
        return level, fname, '', rest.decode('utf-8', 'replace')
      return level, fname, line, f'{fmt} {ptr:08x}: ' + ' '.join(f'{b:02x}' for b in rest)

    values = []
    rest = bytes(args)
    if not isinstance(kinds, (list, tuple)) or not isinstance(fmt, str):
      raise DecodeError('format', f'{where} — malformed logdata entry')
    for i, k in enumerate(kinds):
      if isinstance(k, (list, tuple)):
        if len(k) != 2 or k[0] != 'enum' or not isinstance(k[1], str):
          raise DecodeError('format', f'{where} — unknown argument kind {k!r}')
        if len(rest) < 4:
          raise missing(i, 'enum')
        v, = struct.unpack_from('<i', rest)
        values.append(self.enum_name(k[1], v))
        rest = rest[4:]
      elif k in _INT_KINDS:
        f, n = _INT_KINDS[k]
        if len(rest) < n:
          raise missing(i, k)
        values.append(_Int(struct.unpack_from(f, rest)[0], n))
        rest = rest[n:]
      elif k == 'double':
        if len(rest) < 8:
          raise missing(i, k)
        values.append(struct.unpack_from('<d', rest)[0])
        rest = rest[8:]
      elif k == 'sym':
        if len(rest) < 4:
          raise missing(i, k)
        values.append(self.lookup_sym(struct.unpack_from('<I', rest)[0]))
        rest = rest[4:]
      elif k == 'string':
        z = rest.find(0)
        if z < 0:
          raise missing(i, k)
        values.append(escape_str(rest[:z].decode('utf-8', 'replace')))
        rest = rest[z + 1:]
      elif k == 'bytes':
        values.append(' '.join(f'{b:02x}' for b in rest))
        rest = b''
      else:
        raise DecodeError('format', f'{where} — unknown argument kind {k!r}')
    if rest:
      raise DecodeError('extra_bytes', f'{where} — extra {len(rest)} bytes: {rest.hex()}')
    try:
      text = format_c(fmt, values)
    except ValueError as e:
      raise DecodeError('format', f'{where} — {e}') from None
    return level, fname, line, text
