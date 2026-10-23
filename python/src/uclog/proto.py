# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Framing: COBS, the frame scanner, the 2-byte peek and the frame header (protocol.md §2–§3).

From spec/reference/ucref.py; the scanner works on whole chunks instead of byte by byte.
"""

import struct

__all__ = [
  'cobs_encode', 'cobs_decode', 'CobsError', 'FrameScanner', 'peek', 'mux_decode', 'mux_encode',
  'wire', 'wire_port', 'PULSE', 'MAX_WIRE', 'MAX_FRAME', 'MAX_PAYLOAD', 'MAX_PORT_DATA',
  'TYPE_LOG', 'TYPE_MEM', 'TYPE_RES', 'TYPE_PORT', 'PORT_COMMAND', 'PORT_PLOT', 'PORT_TERMINAL',
  'PORT_DEVICE_INFO', 'PORT_SEMIHOST',
]

# §2.4: a frame on the wire (both delimiters) is at most 1452 bytes.
MAX_WIRE = 1452
MAX_FRAME = MAX_WIRE - 2       # encoded run between the delimiters
MAX_PAYLOAD = 1444             # worst-case COBS(1444 bytes) = 1450
MAX_PORT_DATA = MAX_PAYLOAD - 1

TYPE_LOG = 0
TYPE_MEM = 1
TYPE_RES = 2
TYPE_PORT = 3
TARGET_DIGIT_SHIFT = 20

PORT_COMMAND = 0
PORT_PLOT = 60
PORT_TERMINAL = 61
PORT_DEVICE_INFO = 62
PORT_SEMIHOST = 63

PULSE = b'\x00'


class CobsError(ValueError):
  pass


def cobs_encode(data: bytes) -> bytes:
  """COBS encode. A final 0xFF block that ends exactly at the end of the input is not followed
  by a 0x01 code byte (the decoder infers it)."""
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


def cobs_decode(data: bytes) -> bytes:
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
      raise CobsError(f'code 0x{code:02x} at {i} runs past the end ({n})')
    seg = data[i + 1:end]
    if 0 in seg:
      raise CobsError(f'zero byte inside the block at {i}')
    out += seg
    i = end
    if code < 255 or i == n:
      out.append(0)
  return bytes(out[:-1])


class FrameScanner:
  """Splits a byte stream on 0x00 (§2.2).

  Bytes before the first 0x00 are dropped (they belong to a frame whose start was missed).
  `feed` returns ('frame', encoded) for every non-empty run and ('oversize', n) for a run longer
  than `max_frame` (discarded up to the next 0x00). Empty runs, such as pulses, yield nothing.
  """

  def __init__(self, max_frame=MAX_FRAME):
    self.max_frame = max_frame
    self.buf = bytearray()
    self.discarding = 0
    self.seen_delim = False

  def _part(self, chunk):
    if self.discarding:
      self.discarding += len(chunk)
    else:
      self.buf += chunk
      if len(self.buf) > self.max_frame:
        self.discarding = len(self.buf)
        self.buf.clear()

  def feed(self, data: bytes):
    events = []
    i = 0
    n = len(data)
    if not self.seen_delim:
      j = data.find(0)
      if j < 0:
        return events
      self.seen_delim = True
      i = j
    while i < n:
      j = data.find(0, i)
      if j < 0:
        self._part(data[i:])
        break
      if j > i:
        self._part(data[i:j])
      if self.discarding:
        events.append(('oversize', self.discarding))
        self.discarding = 0
      elif self.buf:
        events.append(('frame', bytes(self.buf)))
      self.buf.clear()
      i = j + 1
    return events


def peek(encoded: bytes):
  """Classify an encoded (still COBS) frame from its first 2 bytes.
  Returns ('pulse',) | ('log', type) | ('port', port)."""
  if len(encoded) == 0 or encoded == b'\x01':
    return ('pulse',)
  b0 = 0 if encoded[0] == 0x01 else encoded[1]
  t = b0 & 3
  if t == TYPE_PORT:
    return ('port', b0 >> 2)
  return ('log', t)


def mux_decode(payload: bytes):
  """Split a decoded frame. Returns ('pulse',) | ('port', port, data) |
  ('log', type, target, addr, args) | ('error', reason)."""
  if len(payload) == 0:
    return ('pulse',)
  t = payload[0] & 3
  if t == TYPE_PORT:
    return ('port', payload[0] >> 2, bytes(payload[1:]))
  if len(payload) < 4:
    return ('error', f'log frame too short ({len(payload)} bytes)')
  addr = struct.unpack('<I', payload[:4])[0]
  return ('log', t, (addr >> TARGET_DIGIT_SHIFT) & 0xF, addr, bytes(payload[4:]))


def mux_encode(port: int, data: bytes) -> bytes:
  if not 0 <= port <= 63:
    raise ValueError(f'port {port} out of range 0..63')
  return bytes(((port << 2) | TYPE_PORT,)) + bytes(data)


def wire(payload: bytes) -> bytes:
  """The complete on-the-wire frame, both delimiters included (§2.1)."""
  return b'\x00' + cobs_encode(payload) + b'\x00'


def wire_port(port: int, data: bytes) -> bytes:
  return wire(mux_encode(port, data))
