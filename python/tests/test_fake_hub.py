# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Connection behaviour against a scripted TCP peer standing in for a hub."""

import socket
import threading
import time

import pytest

import uclog
from uclog import cbor, proto


class Peer:
  """Accepts one client and runs `script(conn)`."""

  def __init__(self, script):
    self.srv = socket.socket()
    self.srv.bind(('127.0.0.1', 0))
    self.srv.listen(1)
    self.port = self.srv.getsockname()[1]
    self.received = bytearray()
    self.t = threading.Thread(target=self._run, args=(script,), daemon=True)
    self.t.start()

  def _run(self, script):
    c, _ = self.srv.accept()
    self.srv.close()
    with c:
      script(self, c)

  def read_frames(self, c, n, timeout=3.0):
    c.settimeout(timeout)
    sc = proto.FrameScanner()
    out = []
    while len(out) < n:
      d = c.recv(4096)
      if not d:
        break
      self.received += d
      out += [proto.cobs_decode(f) for k, f in sc.feed(d) if k == 'frame']
    return out


def test_busy_hub_closes_at_once():
  p = Peer(lambda self, c: c.close())
  with pytest.raises(uclog.HubBusyError):
    uclog.connect(host=f'127.0.0.1:{p.port}')


def test_silent_device():
  p = Peer(lambda self, c: time.sleep(2))
  with pytest.raises(uclog.DeviceNotRespondingError):
    uclog.connect(host=f'127.0.0.1:{p.port}', device_timeout=0.3)


def test_command_pulses_and_wake_gap():
  t0 = []

  def script(self, c):
    c.sendall(proto.PULSE)
    c.settimeout(3)
    first = c.recv(1)  # the client's wake pulse
    t0.append(time.monotonic())
    assert first == b'\0'
    req = self.read_frames(c, 1)[0]
    t0.append(time.monotonic())
    m = proto.mux_decode(req)
    assert m[:2] == ('port', 0)
    cmd = cbor.loads(m[2])
    c.sendall(proto.wire_port(0, b'unrelated text\r\n'))
    c.sendall(proto.wire_port(0, cbor.dumps({'rsp': 'other'})))
    c.sendall(proto.wire_port(0, cbor.dumps({'rsp': cmd['cmd'], 'value': cmd['x'] * 2})))
    end = time.monotonic() + 1.3  # long enough for client pulses
    c.settimeout(0.1)
    while time.monotonic() < end:
      try:
        self.received += c.recv(4096)
      except socket.timeout:
        pass
    c.close()

  p = Peer(script)
  with uclog.connect(host=f'127.0.0.1:{p.port}') as dev:
    rsp = dev.command({'cmd': 'double', 'x': 21}, timeout=2.0)
    assert rsp == {'rsp': 'double', 'value': 42}
    # Frames that were not the reply stay queued.
    assert dev.recv(0, timeout=1) == b'unrelated text\r\n'
    assert cbor.loads(dev.recv(0, timeout=1)) == {'rsp': 'other'}
    assert dev.command({'cmd': 'nobody'}, timeout=0.2) is None
    p.t.join(3)
  assert t0[1] - t0[0] >= 0.045, 'wake gap before the first frame'
  # The request frame has two 00s; the rest are pulses (one per 0.5 s while idle).
  assert p.received.count(0) >= 4, bytes(p.received).hex()


def test_queue_overflow_and_closed():
  def script(self, c):
    for i in range(30):
      c.sendall(proto.wire_port(61, b'%d' % i))
    # Close gracefully: closing with the client's pulses unread would reset the connection,
    # and a reset discards data the client has not read yet.
    c.shutdown(socket.SHUT_WR)
    c.settimeout(0.5)
    try:
      while c.recv(4096):
        pass
    except OSError:
      pass
    c.close()

  p = Peer(script)
  dev = uclog.connect(host=f'127.0.0.1:{p.port}', max_queue=10)
  p.t.join(3)
  end = time.monotonic() + 3
  while not dev.closed and time.monotonic() < end:  # everything read
    time.sleep(0.01)
  got = []
  with pytest.raises(uclog.ConnectionClosedError):
    while True:
      got.append(dev.recv(61, timeout=1))
  assert got == [b'%d' % i for i in range(20, 30)]
  assert dev.dropped[61] == 20
  dev.close()


def test_send_limits():
  p = Peer(lambda self, c: (c.sendall(proto.PULSE), time.sleep(1)))
  with uclog.connect(host=f'127.0.0.1:{p.port}') as dev:
    with pytest.raises(ValueError):
      dev.send(61, b'x' * (proto.MAX_PORT_DATA + 1))
    with pytest.raises(ValueError):
      dev.send(64, b'x')
    dev.send(61, b'x' * proto.MAX_PORT_DATA)


def test_parse_host():
  from uclog.connection import parse_host
  assert parse_host('labpc') == ('labpc', 9000)
  assert parse_host('labpc.local:9001') == ('labpc.local', 9001)
  assert parse_host('tcp://10.0.0.2:9002') == ('10.0.0.2', 9002)
  assert parse_host('[::1]:9003') == ('::1', 9003)
  assert parse_host('[::1]') == ('::1', 9000)


def test_semihost_console_and_duration():
  import struct

  def script(self, c):
    c.sendall(proto.PULSE)
    for ch in b'abc\n':  # unbuffered stderr: one WRITE per byte, no reply
      c.sendall(proto.wire_port(63, bytes([9]) + struct.pack('<I', 2 | 0x80000000) + bytes([ch])))
    c.sendall(proto.wire_port(63, bytes([9]) + struct.pack('<I', 1) + b'out'))
    replies = self.read_frames(c, 1)
    self.reply = proto.mux_decode(replies[0])
    end = time.monotonic() + 1.5
    try:
      while time.monotonic() < end:  # frames without pause, until the client leaves
        c.sendall(proto.wire_port(61, b'x'))
        time.sleep(0.01)
    except OSError:
      pass

  p = Peer(script)
  with uclog.connect(host=f'127.0.0.1:{p.port}') as dev:
    time.sleep(0.3)  # writes not read yet are joined
    assert dev.console(timeout=2) == (2, b'abc\n')
    assert dev.console(timeout=2) == (1, b'out')
    t = time.monotonic()
    n = sum(1 for _ in dev.frames(timeout=1.0, duration=0.5))
    assert n > 10 and time.monotonic() - t < 0.8
  p.t.join(3)
  assert p.reply == ('port', 63, struct.pack('<ii', 3, 0))
