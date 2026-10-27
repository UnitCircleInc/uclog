#!/usr/bin/env python3
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
"""The device library against spec/vectors and the reference (spec/reference/ucref.py), through
the host build (driver.c): framing (COBS), port frames, log argument packing, the device info,
the RX scanner, host presence, the plot messages (§5.6).

  python3 device/test/host/test_vectors.py path/to/uclog_host_test
"""

import json
import re
import struct
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'spec' / 'reference'))
import ucref  # noqa: E402

VECTORS = ROOT / 'spec' / 'vectors'
EXE = None


def vectors(name):
  return json.loads((VECTORS / name).read_text())


class Device:
  """One driver process; commands in, output lines back."""

  def __init__(self):
    self.p = subprocess.Popen([EXE], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)

  def cmd(self, *lines):
    for line in lines:
      self.p.stdin.write(line + '\n')
    self.p.stdin.flush()

  def query(self, line, prefix):
    """Send `line`; return the lines up to the one starting with `prefix` (that one last)."""
    self.cmd(line)
    got = []
    while True:
      s = self.p.stdout.readline()
      if not s:
        raise AssertionError(f'driver exited waiting for {prefix!r}: {got}')
      got.append(s.rstrip('\n'))
      if s.startswith(prefix):
        return got

  def out(self):
    """What the port sent since the last call, as frames (the bytes between 00 delimiters)."""
    line = self.query('out', 'out')[-1]
    return bytes.fromhex(line[4:])

  def connect(self):
    """Boot, then a host pulse: returns the device info frame and the backlog."""
    self.cmd('init', 'ms 1', 'rx 00')
    return self.out()

  def close(self):
    self.p.stdin.close()
    self.p.wait(timeout=5)
    self.p.stdout.close()


def frames(wire: bytes):
  """Split a TX stream on 00; pulses (empty runs) are dropped."""
  return [ucref.cobs_dec(f) for f in wire.split(b'\x00') if f]


def device_version():
  m = re.search(r'#define UCLOG_DEVICE_VERSION "([^"]*)"', (ROOT / 'device/include/log.h').read_text())
  return m[1]


class Tx(unittest.TestCase):
  def setUp(self):
    self.d = Device()
    self.addCleanup(self.d.close)
    self.backlog = frames(self.d.connect())

  def test_device_info_first_then_backlog(self):
    kind = ucref.mux_dec(self.backlog[0])
    self.assertEqual(kind[:2], ('port', 62))
    info = ucref.cbor_loads(kind[2])
    self.assertEqual(info['board'], 'host_test')
    self.assertEqual(info['image_type'], 'app')
    self.assertEqual(info['uclog_version'], device_version())
    self.assertEqual(len(info['app_hash']), 64)
    # log_pre_init() logged before the host came: kept, and sent after the device info.
    self.assertEqual(ucref.mux_dec(self.backlog[1])[0], 'log')

  def test_port_frames_match_the_reference(self):
    for size in [0, 1, 2, 252, 253, 254, 255, 256, 507, 508, 509, 1000, 1442, 1443]:
      for pattern in ['ramp', 'zeros', 'nonzero']:
        data = bytes({'ramp': lambda i: i & 0xff, 'zeros': lambda i: 0,
                      'nonzero': lambda i: 1 + i % 255}[pattern](i) for i in range(size))
        port = (size * 7) % 64
        self.d.cmd(f'tx {port} {data.hex()}')
        self.assertEqual(self.d.out(), ucref.wire(ucref.mux_enc(port, data)), (size, pattern))

  def test_port_frame_vectors(self):
    # (The COBS boundaries, 253-255 and 507-509 bytes, are in the test above: ucref.cobs_enc is
    # checked against cobs.json by spec/reference/check_vectors.py.)
    n = 0
    for c in vectors('peek_mux.json')['cases']:
      if c['mux']['kind'] == 'port':
        payload = bytes.fromhex(c['payload'])
        self.d.cmd(f'tx {payload[0] >> 2} {payload[1:].hex()}')
        self.assertEqual(self.d.out(), b'\x00' + bytes.fromhex(c['encoded']) + b'\x00', c['name'])
        self.assertEqual(ucref.peek(bytes.fromhex(c['encoded'])), ('port', payload[0] >> 2))
        n += 1
    self.assertEqual(n, sum(c['mux']['kind'] == 'port' for c in vectors('peek_mux.json')['cases']))

  def test_log_argument_packing(self):
    self.d.cmd('logs')
    got = [ucref.mux_dec(f) for f in frames(self.d.out())]
    long = b'0123456789abcdef' * 7
    want = [
      (0, b''),
      (0, struct.pack('<iII', -123456, 4000000000, 0xbeef)),
      (0, struct.pack('<iIii', -2, 250, 1, 65)),
      (0, struct.pack('<qQ', -9000000000000000000, 18000000000000000000)),
      (0, struct.pack('<dd', 1.5, 3.14159265358979)),
      (0, struct.pack('<d', 2.5)),               # long double: sent as a double
      (0, b'hello\0\0'),
      (0, struct.pack('<I', 0x2000abcd)),
      (0, struct.pack('<i', 5)),
      (0, struct.pack('<14i', *range(1, 15))),
      (0, long[:ucref_max_log() - 4 - 1] + b'\0'),  # truncated to the frame
      (0, b''), (0, b''), (0, b''),
      (1, None),                                  # LOG_MEM: checked below
    ]
    self.assertEqual(len(got), len(want))
    for i, ((t, args), g) in enumerate(zip(want, got)):
      self.assertEqual(g[0], 'log', i)
      kind, ty, target, addr, data = g
      self.assertEqual(ty, t, i)
      self.assertEqual(addr & 3, t, i)
      self.assertNotEqual(addr & ~3, 0, i)
      if args is not None:
        self.assertEqual(data, args, i)
    mem = got[-1][4]
    self.assertEqual(len(mem), 4 + 20)               # data address, then the bytes
    self.assertEqual(mem[4:8], bytes([0, 1, 2, 0x7f]))
    # Every log frame is at most LOG_MAX_LOG_SIZE bytes before COBS.
    self.assertTrue(all(len(g[4]) + 4 <= ucref_max_log() for g in got))


def ucref_max_log():
  return 100  # LOG_MAX_LOG_SIZE (log.h)


class Rx(unittest.TestCase):
  def test_scanner_vectors(self):
    for c in vectors('scanner.json')['cases']:
      if c['max_frame'] != 1450:
        continue
      d = Device()
      try:
        d.cmd('init', 'ms 1', *[f'notify {p}' for p in range(64) if p != 62])
        want, errors = [], 0
        for e in c['events']:
          if 'oversize' in e:
            errors += 1
            continue
          payload = ucref.cobs_dec(bytes.fromhex(e['frame']))
          kind = ucref.mux_dec(payload)
          if kind[0] == 'port' and kind[1] != 62:
            want.append(f'frame {kind[1]} {kind[2].hex()}')
          elif kind[0] == 'port' and kind[2]:
            errors += 1                             # port 62 from the host: only the empty request
          elif kind[0] in ('log', 'error'):
            errors += 1                             # a host must not send log frames
        for chunk in c['chunks']:
          d.cmd(f'rx {chunk}')
        lines = d.query('stats', 'stats')
        got = [s for s in lines if s.startswith('frame')]
        stats = lines[-1].split()
        self.assertEqual(got, want, c['name'])
        self.assertEqual(int(stats[3]), errors, c['name'])
      finally:
        d.close()

  def test_device_info_request_f7(self):
    d = Device()
    self.addCleanup(d.close)
    d.connect()
    req = bytes.fromhex(vectors('device_info.json')['request']['wire'])
    d.cmd(f'rx {req.hex()}')
    f = frames(d.out())
    self.assertEqual(ucref.mux_dec(f[0])[:2], ('port', 62))

  def test_bad_cobs_is_counted(self):
    d = Device()
    self.addCleanup(d.close)
    d.connect()
    d.cmd('rx 0003fb0000')                       # code 03 but only one byte follows
    self.assertEqual(d.query('stats', 'stats')[-1].split()[3], '1')


class Plots(unittest.TestCase):
  def test_plot_messages(self):
    d = Device()
    self.addCleanup(d.close)
    d.connect()
    lines = d.query('plots\nout', 'out')
    self.assertEqual([s for s in lines if s.startswith('ret')], ['ret 1'] * 5 + ['ret 0'] * 2)
    msgs = []
    for f in frames(bytes.fromhex(lines[-1][4:])):
      kind = ucref.mux_dec(f)
      self.assertEqual(kind[:2], ('port', 60))
      msgs.append(ucref.cbor_loads(kind[2]))
    self.assertEqual(msgs, [
      {'plot': 'weather', 'xlabel': 's', 'ylabel': ['C', '%'], 'yser': ['inside', 'outside'],
       'y2ser': ['humidity'], 'y2limits': [0.0, 100.0], 'y2props': ['dashed'], 'xscroll': 500},
      {'plot': 'weather', 'x': [1.0], 'y11': [21.5], 'y12': [4.25]},
      {'plot': 'weather', 'x': [1.0, 2.0, 3.0], 'y21': [40.0, 41.5, 43.0]},
      {'plot': 'weather', 'y12': [40.0]},
      {'clear_plot': 'weather'},
    ])


class Presence(unittest.TestCase):
  def test_pulses_timeout_and_return(self):
    d = Device()
    self.addCleanup(d.close)
    d.connect()
    d.cmd('ms 600')                              # 0.5 s of TX idle: a pulse
    self.assertEqual(d.out(), b'\x00')
    d.cmd('ms 2100')                             # 2 s without RX: the host is gone
    self.assertEqual(d.query('host', 'host')[-1], 'host 0')
    d.cmd('logs')
    self.assertEqual(d.out(), b'')               # kept until a host returns
    d.cmd('ms 2200', 'rx 00')
    f = frames(d.out())
    self.assertEqual(ucref.mux_dec(f[0])[:2], ('port', 62))   # device info first
    self.assertEqual(len(f), 1 + 15)              # then the 15 logs


if __name__ == '__main__':
  EXE = sys.argv.pop(1)
  unittest.main()
