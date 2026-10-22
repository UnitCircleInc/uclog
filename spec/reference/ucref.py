# © 2026 Unit Circle Inc.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Executable reference for spec/protocol.md.

Standard library only (no cbor2/crc32c), so it runs anywhere Python 3.9+ does. It is
used to generate spec/vectors/*.json and to check them. It is not a client: there is no
I/O here. Section numbers in comments refer to spec/protocol.md.
"""

import math
import re
import struct

# ---------------------------------------------------------------------------
# §2 COBS
# ---------------------------------------------------------------------------

def cobs_enc(data: bytes) -> bytes:
  """COBS encode (Cheshire & Baker). A final 0xFF block that ends exactly at the end of
  the input is not followed by a 0x01 code byte (the decoder infers it)."""
  out = bytearray()
  data = bytes(data) + b'\0'
  while data:
    i = data.index(b'\0')
    if i >= 254:
      out.append(255)
      out += data[:254]
      data = data[254:]
      if data == b'\0':
        break
    else:
      out.append(i + 1)
      out += data[:i]
      data = data[i + 1:]
  return bytes(out)


class CobsError(ValueError):
  pass


def cobs_dec(data: bytes) -> bytes:
  """COBS decode. Raises CobsError on a zero byte or a code that runs past the end."""
  out = bytearray()
  i = 0
  n = len(data)
  if n == 0:
    raise CobsError('empty')
  while i < n:
    code = data[i]
    if code == 0:
      raise CobsError(f'zero byte at {i}')
    end = i + code
    if end > n:
      raise CobsError(f'code 0x{code:02x} at {i} runs past end ({n})')
    seg = data[i + 1:end]
    if 0 in seg:
      raise CobsError(f'zero byte inside block at {i}')
    out += seg
    i = end
    if code < 255 or i == n:
      out.append(0)
  # Remove the implicit trailing zero. A trailing 0xFF block adds it above too.
  return bytes(out[:-1])


# ---------------------------------------------------------------------------
# §2.2 Frame scanner
# ---------------------------------------------------------------------------

MAX_FRAME = 4096


class FrameScanner:
  """Splits a byte stream on 0x00. Bytes before the first 0x00 are dropped (they belong
  to a frame whose start was missed). Then it emits ('frame', encoded_bytes) for every
  non-empty run and ('oversize', n) when a run exceeds max_frame (discarded up to the
  next 0x00). Empty runs (00 00) are ignored: every frame starts and ends with 0x00, so
  back-to-back frames always produce them. A lone 0x00 pulse therefore yields no event;
  receivers use "any byte received" for liveness (§2.3)."""

  def __init__(self, max_frame=MAX_FRAME):
    self.max_frame = max_frame
    self.buf = bytearray()
    self.discarding = 0
    self.seen_delim = False

  def feed(self, data: bytes):
    events = []
    for b in data:
      if b == 0:
        if self.discarding:
          events.append(('oversize', self.discarding))
          self.discarding = 0
        elif self.buf:
          events.append(('frame', bytes(self.buf)))
        self.buf.clear()
        self.seen_delim = True
      elif not self.seen_delim:
        continue  # not yet in sync: bytes before the first 0x00 belong to a partial frame
      elif self.discarding:
        self.discarding += 1
      else:
        self.buf.append(b)
        if len(self.buf) > self.max_frame:
          self.discarding = len(self.buf)
          self.buf.clear()
    return events


# ---------------------------------------------------------------------------
# §3 Frame header, 2-byte peek, mux
# ---------------------------------------------------------------------------

TYPE_LOG = 0
TYPE_MEM = 1
TYPE_RES = 2
TYPE_PORT = 3
TARGET_DIGIT_SHIFT = 20


def peek(encoded: bytes):
  """Classify an encoded (still COBS) frame from at most its first 2 bytes.
  Returns ('pulse',) | ('log', type) | ('port', port)."""
  if len(encoded) == 0 or encoded == b'\x01':
    return ('pulse',)
  if encoded[0] == 0x01:
    b0 = 0x00
  else:
    b0 = encoded[1]
  t = b0 & 3
  if t == TYPE_PORT:
    return ('port', b0 >> 2)
  return ('log', t)


def mux_dec(payload: bytes):
  """Split a decoded frame. Returns ('pulse',) | ('port', port, data) |
  ('log', type, target, addr, args) | ('error', reason)."""
  if len(payload) == 0:
    return ('pulse',)
  t = payload[0] & 3
  if t == TYPE_PORT:
    return ('port', payload[0] >> 2, payload[1:])
  if len(payload) < 4:
    return ('error', f'log frame too short ({len(payload)} bytes)')
  addr = struct.unpack('<I', payload[:4])[0]
  return ('log', t, (addr >> TARGET_DIGIT_SHIFT) & 0xF, addr, payload[4:])


def mux_enc(port: int, data: bytes) -> bytes:
  return bytes(((port << 2) | TYPE_PORT,)) + data


def wire(payload: bytes) -> bytes:
  """Complete on-the-wire encoding of one frame, including the leading 0x00 (§2.1)."""
  return b'\x00' + cobs_enc(payload) + b'\x00'


# ---------------------------------------------------------------------------
# CRC32C (Castagnoli) — legacy port 3 only
# ---------------------------------------------------------------------------

def _crc32c_table():
  t = []
  for i in range(256):
    c = i
    for _ in range(8):
      c = (c >> 1) ^ 0x82F63B78 if c & 1 else c >> 1
    t.append(c)
  return t


_CRC32C = _crc32c_table()


def crc32c(data: bytes) -> int:
  c = 0xFFFFFFFF
  for b in data:
    c = _CRC32C[(c ^ b) & 0xFF] ^ (c >> 8)
  return c ^ 0xFFFFFFFF


# ---------------------------------------------------------------------------
# Minimal CBOR (RFC 8949) — enough for every payload in this protocol
# ---------------------------------------------------------------------------

def _head(major, n):
  if n < 24:
    return bytes(((major << 5) | n,))
  for ai, fmt in ((24, '>B'), (25, '>H'), (26, '>I'), (27, '>Q')):
    if n < (1 << (8 * struct.calcsize(fmt))):
      return bytes(((major << 5) | ai,)) + struct.pack(fmt, n)
  raise ValueError('integer too large')


def cbor_dumps(v) -> bytes:
  """Deterministic encoder: shortest heads, map keys in insertion order, floats as
  float64 unless exactly representable as float32 (to match cbor2's canonical=False
  output for the values used in vectors, floats are always float64)."""
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
  if isinstance(v, (bytes, bytearray)):
    return _head(2, len(v)) + bytes(v)
  if isinstance(v, str):
    b = v.encode('utf-8')
    return _head(3, len(b)) + b
  if isinstance(v, (list, tuple)):
    return _head(4, len(v)) + b''.join(cbor_dumps(x) for x in v)
  if isinstance(v, dict):
    return _head(5, len(v)) + b''.join(cbor_dumps(k) + cbor_dumps(x) for k, x in v.items())
  raise TypeError(f'cannot encode {type(v)}')


class CborError(ValueError):
  pass


def _freeze(k):
  if isinstance(k, list):
    return tuple(_freeze(x) for x in k)
  return k


def cbor_loads(data: bytes):
  v, i = _cbor_item(memoryview(bytes(data)), 0)
  if i != len(data):
    raise CborError(f'{len(data) - i} trailing bytes')
  return v


def _cbor_item(d, i):
  if i >= len(d):
    raise CborError('truncated')
  ib = d[i]
  major, ai = ib >> 5, ib & 31
  i += 1
  if major == 7:
    if ai == 20: return False, i
    if ai == 21: return True, i
    if ai in (22, 23): return None, i
    if ai == 25: return struct.unpack('>e', d[i:i + 2])[0], i + 2
    if ai == 26: return struct.unpack('>f', d[i:i + 4])[0], i + 4
    if ai == 27: return struct.unpack('>d', d[i:i + 8])[0], i + 8
    raise CborError(f'unsupported simple value {ai}')
  if ai < 24:
    n = ai
  elif ai in (24, 25, 26, 27):
    size = 1 << (ai - 24)
    if i + size > len(d):
      raise CborError('truncated')
    n = int.from_bytes(d[i:i + size], 'big')
    i += size
  elif ai == 31 and major in (2, 3, 4, 5):
    n = None  # indefinite
  else:
    raise CborError(f'bad additional info {ai}')
  if major == 0:
    return n, i
  if major == 1:
    return -1 - n, i
  if major in (2, 3):
    if n is None:
      parts = []
      while d[i] != 0xFF:
        p, i = _cbor_item(d, i)
        parts.append(p)
      i += 1
      return (b''.join(parts) if major == 2 else ''.join(parts)), i
    if i + n > len(d):
      raise CborError('truncated')
    raw = bytes(d[i:i + n])
    return (raw if major == 2 else raw.decode('utf-8')), i + n
  if major == 4:
    out = []
    while (n is None and d[i] != 0xFF) or (n is not None and len(out) < n):
      x, i = _cbor_item(d, i)
      out.append(x)
    if n is None:
      i += 1
    return out, i
  if major == 5:
    out = {}
    count = 0
    while (n is None and d[i] != 0xFF) or (n is not None and count < n):
      k, i = _cbor_item(d, i)
      x, i = _cbor_item(d, i)
      out[_freeze(k)] = x
      count += 1
    if n is None:
      i += 1
    return out, i
  if major == 6:
    return _cbor_item(d, i)  # tags are ignored
  raise CborError('unreachable')


# ---------------------------------------------------------------------------
# §6 Logdata and log decoding
# ---------------------------------------------------------------------------

LEVELS = {'0': 'DEBUG', '1': 'INFO', '2': 'WARN', '3': 'ERROR', '4': 'FATAL', '5': 'PANIC'}

ARG_SIZES = {'int32': 4, 'uint32': 4, 'int64': 8, 'uint64': 8, 'double': 8, 'pointer': 4, 'sym': 4}


class LogData:
  """Parsed .logdata CBOR (§6.1)."""

  def __init__(self, data: bytes):
    a = cbor_loads(data)
    self.enums = a['enums']
    self.tdenums = a['tdenums']
    self.vars = a['vars']
    self.fns = a['fns']
    self.saddr = a['saddr']
    self.fmts = a['fmts']

  @property
  def target(self):
    return (self.saddr >> TARGET_DIGIT_SHIFT) & 0xF

  def lookup_sym(self, a):
    a2 = a & ~1
    for (low, size), name in self.fns.items():
      if low <= a2 < low + size:
        return f'{name}+0x{a2 - low:x}'
    best = None
    for addr, name in self.vars.items():
      off = a - addr
      if 0 <= off < 0x3000 and (best is None or off < best[1]):
        best = (name, off)
    if best:
      return f'{best[0]}+0x{best[1]:x}'
    return f'0x{a:08x}'

  def enum_name(self, name, v):
    if name in self.enums:
      return self.enums[name].get(v, f'<{name}:{v}>')
    if name in self.tdenums:
      return self.tdenums[name].get(v, f'<{name}:{v}>')
    return f'<!{name}:{v}>'


# Conversion specifications that appear in the 'clean' format string (§6.2).
# Length modifiers are accepted and ignored: argument sizes come from the kinds list, and
# logdata generators rewrite '%llu' as '%lu' (so '%lu' may carry a uint64).
_SPEC = re.compile(r'%(?P<flags>[#+\- 0]*)(?P<width>[0-9]*)(?:\.(?P<prec>[0-9]*))?'
                   r'(?P<len>hh|h|ll|l|z|j|t|L)?(?P<conv>[diouxXcfFeEgGaAs%])')


def _take(args, kind):
  size = ARG_SIZES.get(kind)
  if kind == 'string':
    try:
      i = args.index(0)
    except ValueError:
      raise DecodeError('missing string')
    return args[:i].decode('utf-8', 'replace'), args[i + 1:]
  if kind == 'bytes':
    return args, b''
  if size is None:
    raise DecodeError(f'unknown argument kind {kind!r}')
  if len(args) < size:
    raise DecodeError(f'missing {kind}')
  fmt = {'int32': '<i', 'uint32': '<I', 'int64': '<q', 'uint64': '<Q', 'double': '<d',
         'pointer': '<I', 'sym': '<I'}[kind]
  return struct.unpack(fmt, args[:size])[0], args[size:]


class DecodeError(ValueError):
  pass


def format_c(fmt: str, values) -> str:
  """Format the 'clean' string with C-printf semantics (§6.2). Mirrors Python '%' for
  the conversions used by logdata, which is what the reference output is checked
  against; exact parity in other languages is not required (README §5.4)."""
  out = []
  pos = 0
  vi = 0
  for m in _SPEC.finditer(fmt):
    out.append(fmt[pos:m.start()])
    pos = m.end()
    if m.group('conv') == '%':
      out.append('%')
      continue
    if vi >= len(values):
      raise DecodeError('format has more conversions than values')
    v = values[vi]
    vi += 1
    spec = '%' + m.group('flags') + m.group('width')
    if m.group('prec') is not None:
      spec += '.' + m.group('prec')
    out.append((spec + m.group('conv')) % (v,))
  out.append(fmt[pos:])
  if vi != len(values):
    raise DecodeError('format has fewer conversions than values')
  return ''.join(out)


_ESC = {'\\': '\\\\', '\n': '\\n', '\r': '\\r', '\t': '\\t'}


def escape_str(v: str) -> str:
  """Escape a %s string argument (§6.3): backslash, \\n, \\r, \\t, and other control
  characters as \\xHH. Everything else (quotes, non-ASCII) is kept."""
  out = []
  for ch in v:
    if ch in _ESC:
      out.append(_ESC[ch])
    elif ord(ch) < 0x20 or ord(ch) == 0x7F:
      out.append(f'\\x{ord(ch):02x}')
    else:
      out.append(ch)
  return ''.join(out)


def decode_log(ld: LogData, addr: int, args: bytes):
  """Decode one type-0/1 log frame. Returns dict(level, file, line, text) or raises
  DecodeError (the caller turns it into an error record, §7)."""
  kind = addr & 3
  key = addr & ~3
  entry = ld.fmts.get(key)
  if entry is None:
    raise DecodeError(f'unknown format addr 0x{key:08x}')
  if len(entry) != 5:
    raise DecodeError(f'format addr 0x{key:08x} has no format string')
  level, fname, line, clean, kinds = entry
  if kind == TYPE_MEM:
    ptr, rest = _take(args, 'uint32')
    if line == '<zephyr>':
      text = rest.decode('utf-8', 'replace')
      line = ''
    else:
      text = f'{clean} {ptr:08x}: ' + ' '.join(f'{b:02x}' for b in rest)
  else:
    values = []
    rest = args
    for k in kinds:
      if isinstance(k, (list, tuple)) and len(k) == 2 and k[0] == 'enum':
        v, rest = _take(rest, 'int32')
        values.append(ld.enum_name(k[1], v))
      elif k == 'sym':
        v, rest = _take(rest, 'uint32')
        values.append(ld.lookup_sym(v))
      elif k == 'string':
        v, rest = _take(rest, 'string')
        values.append(escape_str(v))
      else:
        v, rest = _take(rest, k)
        values.append(v)
    if rest:
      raise DecodeError(f'extra {len(rest)} bytes: {rest.hex()}')
    text = format_c(clean, values)
  return {'level': LEVELS.get(level, f'<bad level {level}>'), 'file': fname,
          'line': line, 'text': text}


# ---------------------------------------------------------------------------
# §5.5 Semihost (port 63) codec
# ---------------------------------------------------------------------------

SH_OPS = {
  0: 'SYSTEM', 1: 'RENAME', 2: 'ISATTY', 3: 'UNLINK', 4: 'LINK', 5: 'STAT', 6: 'FSTAT',
  7: 'CLOSE', 8: 'OPEN', 9: 'WRITE', 10: 'LSEEK', 11: 'READ', 12: 'MKDIR', 13: 'TIMEOFDAY',
  0x10: 'SYS_CLOCK', 0x11: 'SYS_TIME', 0x30: 'SYS_ELAPSED', 0x31: 'SYS_TICKFREQ',
}
TIME_OPS = {13, 0x10, 0x11, 0x30, 0x31}

# newlib <sys/errno.h>
NEWLIB_ERRNO = {
  'EPERM': 1, 'ENOENT': 2, 'EINTR': 4, 'EIO': 5, 'EBADF': 9, 'EAGAIN': 11, 'ENOMEM': 12,
  'EACCES': 13, 'EEXIST': 17, 'ENOTDIR': 20, 'EISDIR': 21, 'EINVAL': 22, 'EMFILE': 24,
  'ENOSPC': 28, 'ESPIPE': 29, 'EROFS': 30, 'ENAMETOOLONG': 91, 'ENOSYS': 88,
  'ENOTEMPTY': 90,
}

WRITE_NO_REPLY = 0x80000000


# Ops whose success reply has a fixed size; their error replies are zero padded to it (§5.5.4).
FIXED_REPLY_SIZE = {6: 48}


def sh_error(errno_name: str, op: int = None) -> bytes:
  """Error reply <i32 -1, i32 errno> (§5.5.4), zero padded for fixed-size replies."""
  r = struct.pack('<ii', -1, NEWLIB_ERRNO[errno_name])
  return r + b'\0' * (FIXED_REPLY_SIZE.get(op, 8) - 8)


def sh_reply_time(op: int, *, mono_ns=None, qpc=None, qpf=None, wall=None) -> bytes:
  """Encode a time-op reply (§5.5.3). Exactly one of mono_ns or (qpc, qpf) describes the
  monotonic source; wall is float seconds since the epoch."""
  if qpc is not None:
    ticks, freq = qpc, qpf
  else:
    ticks, freq = mono_ns, 1_000_000_000
  if op == 0x10:
    return struct.pack('<I', (ticks * 100 // freq) & 0xFFFFFFFF)
  if op == 0x11:
    return struct.pack('<q', math.floor(wall))
  if op == 0x30:
    return struct.pack('<Q', ticks & 0xFFFFFFFFFFFFFFFF)
  if op == 0x31:
    return struct.pack('<Q', freq)
  if op == 13:
    sec = math.floor(wall)
    usec = min(int((wall - sec) * 1e6 + 0.5), 999_999)
    return struct.pack('<QI', sec, usec)
  raise ValueError(op)


def sh_parse_request(frame: bytes):
  """Split a port-63 request into (op, name, fields). Returns fields as a dict; raises
  ValueError on a malformed request. Unknown ops return name None."""
  if not frame:
    raise ValueError('empty request')
  op, a = frame[0], frame[1:]
  name = SH_OPS.get(op)
  f = {}
  if name in ('ISATTY', 'FSTAT', 'CLOSE'):
    if len(a) != 4: raise ValueError('bad size')
    f['fid'] = struct.unpack('<i', a)[0]
  elif name in ('OPEN', 'MKDIR'):
    if len(a) < 4: raise ValueError('bad size')
    f['flags' if name == 'OPEN' else 'mode'] = struct.unpack('<i', a[:4])[0]
    f['path'] = a[4:].decode('utf-8')
  elif name == 'WRITE':
    if len(a) < 4: raise ValueError('bad size')
    raw = struct.unpack('<I', a[:4])[0]
    f['fid'] = raw & 0x7FFFFFFF
    f['reply'] = not (raw & WRITE_NO_REPLY)
    f['data'] = a[4:]
  elif name == 'READ':
    if len(a) != 8: raise ValueError('bad size')
    f['fid'], f['len'] = struct.unpack('<iI', a)
  elif name in ('TIMEOFDAY', 'SYS_CLOCK', 'SYS_TIME', 'SYS_ELAPSED', 'SYS_TICKFREQ'):
    if a: raise ValueError('time ops take no arguments')
  else:
    f['args'] = a
  return op, name, f


# ---------------------------------------------------------------------------
# §4.3 HID chunking
# ---------------------------------------------------------------------------

HID_IN = 0xA1
HID_OUT = 0xA2
HID_PAYLOAD = 63


def hid_reports(data: bytes):
  """Host → device output reports: report id 0xA2 + up to 63 payload bytes, zero padded
  to 63 (padding is 0x00 = frame delimiters, which the scanner ignores)."""
  out = []
  for k in range(0, len(data), HID_PAYLOAD):
    chunk = data[k:k + HID_PAYLOAD]
    out.append(bytes((HID_OUT,)) + chunk + b'\0' * (HID_PAYLOAD - len(chunk)))
  return out


def stlink_split(data: bytes):
  """STLINK VCP workaround (§4.1): a write whose length is a multiple of 8 is split so
  the last write is 1 byte."""
  if len(data) > 1 and len(data) % 8 == 0:
    return [data[:-1], data[-1:]]
  return [data]
