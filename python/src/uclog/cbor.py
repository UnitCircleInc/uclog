# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Minimal CBOR (RFC 8949): enough for every payload in the ucLog protocol.

From spec/reference/ucref.py. Standard library only, so the package has no dependencies.
"""

import struct

__all__ = ['dumps', 'loads', 'CborError']


class CborError(ValueError):
  pass


def _head(major, n):
  if n < 24:
    return bytes(((major << 5) | n,))
  for ai, fmt in ((24, '>B'), (25, '>H'), (26, '>I'), (27, '>Q')):
    if n < (1 << (8 * struct.calcsize(fmt))):
      return bytes(((major << 5) | ai,)) + struct.pack(fmt, n)
  raise ValueError('integer too large for CBOR')


def dumps(v) -> bytes:
  """Encode: shortest heads, map keys in insertion order, floats as float64."""
  if v is None:
    return b'\xf6'
  if v is True:
    return b'\xf5'
  if v is False:
    return b'\xf4'
  if isinstance(v, int):
    return _head(0, v) if v >= 0 else _head(1, -1 - v)
  if isinstance(v, float):
    return b'\xfb' + struct.pack('>d', v)
  if isinstance(v, (bytes, bytearray, memoryview)):
    return _head(2, len(v)) + bytes(v)
  if isinstance(v, str):
    b = v.encode('utf-8')
    return _head(3, len(b)) + b
  if isinstance(v, (list, tuple)):
    return _head(4, len(v)) + b''.join(dumps(x) for x in v)
  if isinstance(v, dict):
    return _head(5, len(v)) + b''.join(dumps(k) + dumps(x) for k, x in v.items())
  raise TypeError(f'cannot encode {type(v).__name__} as CBOR')


def loads(data: bytes):
  """Decode one item. Array map keys become tuples; tags are ignored."""
  d = memoryview(bytes(data))
  v, i = _item(d, 0)
  if i != len(d):
    raise CborError(f'{len(d) - i} trailing bytes')
  return v


def _freeze(k):
  if isinstance(k, list):
    return tuple(_freeze(x) for x in k)
  return k


def _byte(d, i):
  if i >= len(d):
    raise CborError('truncated')
  return d[i]


def _item(d, i):
  ib = _byte(d, i)
  major, ai = ib >> 5, ib & 31
  i += 1
  if major == 7:
    if ai == 20:
      return False, i
    if ai == 21:
      return True, i
    if ai in (22, 23):
      return None, i
    size = {25: 2, 26: 4, 27: 8}.get(ai)
    if size is None:
      raise CborError(f'unsupported simple value {ai}')
    if i + size > len(d):
      raise CborError('truncated')
    return struct.unpack({2: '>e', 4: '>f', 8: '>d'}[size], d[i:i + size])[0], i + size
  if ai < 24:
    n = ai
  elif ai in (24, 25, 26, 27):
    size = 1 << (ai - 24)
    if i + size > len(d):
      raise CborError('truncated')
    n = int.from_bytes(d[i:i + size], 'big')
    i += size
  elif ai == 31 and major in (2, 3, 4, 5):
    n = None  # indefinite length
  else:
    raise CborError(f'bad additional info {ai}')
  if major == 0:
    return n, i
  if major == 1:
    return -1 - n, i
  if major in (2, 3):
    if n is None:
      parts = []
      while _byte(d, i) != 0xFF:
        p, i = _item(d, i)
        parts.append(p)
      i += 1
      return (b''.join(parts) if major == 2 else ''.join(parts)), i
    if i + n > len(d):
      raise CborError('truncated')
    raw = bytes(d[i:i + n])
    try:
      return (raw if major == 2 else raw.decode('utf-8')), i + n
    except UnicodeDecodeError as e:
      raise CborError(f'invalid UTF-8 in text: {e}') from None
  if major == 4:
    out = []
    while (n is None and _byte(d, i) != 0xFF) or (n is not None and len(out) < n):
      x, i = _item(d, i)
      out.append(x)
    if n is None:
      i += 1
    return out, i
  if major == 5:
    out = {}
    count = 0
    while (n is None and _byte(d, i) != 0xFF) or (n is not None and count < n):
      k, i = _item(d, i)
      x, i = _item(d, i)
      out[_freeze(k)] = x
      count += 1
    if n is None:
      i += 1
    return out, i
  return _item(d, i)  # major 6: tag, ignored
