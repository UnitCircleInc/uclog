# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Connection behaviour against a scripted TCP peer standing in for a hub."""

import socket
import struct
import threading
import time

import pytest

import uclog
from uclog import cbor, proto

# What a hub with an empty backlog sends while the device is alive: the end-of-backlog pulse,
# then a pulse (protocol.md §8). The second 00 shows the device is open.
HELLO = b'\x00\x00'


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
    c.sendall(HELLO)
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
  p = Peer(lambda self, c: (c.sendall(HELLO), time.sleep(1)))
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
    c.sendall(HELLO)
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


# --------------------------------------------------------------------------- review fixes

def _logdata(text, saddr=0x00100000):
  """A one-format logdata (target 1): `text %d`."""
  return cbor.dumps({'saddr': saddr, 'fmts': {saddr: ['1', 'a.c', '1', text + ' %d', ['int32']]},
                     'enums': {}, 'tdenums': {}, 'fns': {}, 'vars': {}})


def _info(h):
  return proto.wire_port(62, cbor.dumps({'board': 'b', 'app_hash': h}))


def _log(v, addr=0x00100000):
  return proto.wire(struct.pack('<Ii', addr, v))


def _linger(c, seconds=3.0):
  """Keep the connection up (reading the client's pulses) until the client leaves."""
  c.settimeout(0.1)
  end = time.monotonic() + seconds
  while time.monotonic() < end:
    try:
      if not c.recv(4096):
        return
    except socket.timeout:
      pass
    except OSError:
      return


def test_connect_waits_for_the_device_not_the_replay():
  # A hub still opening the device sends only the end-of-backlog pulse (BLE takes ~5 s).
  def script(self, c):
    c.sendall(b'\x00')
    time.sleep(0.6)
    c.sendall(b'\x00' + _info(b'\x01' * 64)[1:])  # the device: a pulse, then its device info
    _linger(c)

  p = Peer(script)
  t = time.monotonic()
  with uclog.connect(host=f'127.0.0.1:{p.port}', device_timeout=3) as dev:
    assert time.monotonic() - t >= 0.5
    assert dev.device_info['board'] == 'b'


def test_replay_end_alone_is_device_not_responding():
  def script(self, c):
    c.sendall(_info(b'\x01' * 64) + b'\x00')  # cached device info, then the end of the replay
    time.sleep(1.5)

  p = Peer(script)
  with pytest.raises(uclog.DeviceNotRespondingError, match='has not opened the device'):
    uclog.connect(host=f'127.0.0.1:{p.port}', device_timeout=0.5)


def test_device_info_waits_for_the_rest_of_device_timeout():
  def script(self, c):
    c.sendall(HELLO)
    time.sleep(2.5)  # longer than device_info's usual 2 s
    c.sendall(_info(b'\x02' * 64))
    _linger(c)

  p = Peer(script)
  with uclog.connect(host=f'127.0.0.1:{p.port}', device_timeout=4) as dev:
    assert dev.device_info is not None


def test_backlog_logs_keep_their_image_logdata(tmp_path, monkeypatch):
  ha, hb, hc = b'\xaa' * 64, b'\xbb' * 64, b'\xcc' * 64
  (tmp_path / (ha.hex() + '.logdata')).write_bytes(_logdata('image A'))
  (tmp_path / (hb.hex() + '.logdata')).write_bytes(_logdata('image B'))
  real = uclog.Resolver.resolve

  def slow(self, h):
    if h == ha:
      time.sleep(0.5)  # A resolves after B's device info (and B's logs) have arrived
    return real(self, h)

  monkeypatch.setattr(uclog.Resolver, 'resolve', slow)

  def script(self, c):
    c.sendall(_info(ha) + _log(1) + _info(hb) + _log(2) + _info(hc) + _log(3) + HELLO)
    _linger(c)

  p = Peer(script)
  with uclog.connect(host=f'127.0.0.1:{p.port}', cache_dir=tmp_path) as dev:
    recs = [r for _, r in zip(range(3), dev.logs(timeout=3))]
  assert [r.text for r in recs[:2]] == ['image A 1', 'image B 2']
  # A new image without logdata does not keep using the previous image's decoders.
  assert recs[2].error == 'no_logdata'


def test_resolver_failure_does_not_stall_logs(tmp_path, monkeypatch):
  def boom(self, h):
    raise RuntimeError('resolver bug')

  monkeypatch.setattr(uclog.Resolver, 'resolve', boom)

  def script(self, c):
    c.sendall(_info(b'\x03' * 64) + _log(1) + HELLO)
    _linger(c)

  p = Peer(script)
  with uclog.connect(host=f'127.0.0.1:{p.port}', cache_dir=tmp_path) as dev:
    r = next(dev.logs(timeout=3))
    assert r.error == 'no_logdata'
    assert any('resolver bug' in n for n in dev.notes)


def test_malformed_frames_do_not_close_the_connection(tmp_path):
  bad_infos = [b'\xa1\xa0\x01',          # map key is a map (unhashable)
               b'\x5f\x61a\xff',         # text chunk inside indefinite bytes
               b'\x81' * 200 + b'\x00',  # nested too deep
               b'\xbf\x01']              # truncated indefinite map
  bad_logdata = cbor.dumps({'saddr': 0x00100000, 'enums': {}, 'tdenums': {}, 'fns': {}, 'vars': {},
                            'fmts': {0x00100000: ['1', 'a.c', '1', '%s', [['enum']]]}})
  ld = tmp_path / 'bad.logdata'
  ld.write_bytes(bad_logdata)

  def script(self, c):
    c.sendall(HELLO)
    for b in bad_infos:
      c.sendall(proto.wire_port(62, b))
    c.sendall(proto.wire_port(0, b'\xff not cbor'))
    c.sendall(_log(1))
    req = self.read_frames(c, 1)
    m = cbor.loads(proto.mux_decode(req[0])[2])
    c.sendall(proto.wire_port(0, cbor.dumps({'rsp': m['cmd']})))
    c.sendall(proto.wire_port(61, b'still here'))
    _linger(c)

  p = Peer(script)
  with uclog.connect(host=f'127.0.0.1:{p.port}', logdata=ld) as dev:
    assert dev.command({'cmd': 'ping'}, timeout=3) == {'rsp': 'ping'}
    assert dev.recv(61, timeout=3) == b'still here'
    recs = list(dev.logs(timeout=0.3))
    assert [r.error for r in recs] == ['bad_cbor'] * 4 + ['format']
    assert not dev.closed


def test_close_flushes_queued_sends():
  got = []

  def script(self, c):
    c.settimeout(3)
    sc = proto.FrameScanner()
    try:
      while True:
        d = c.recv(65536)
        if not d:
          break
        got.extend(proto.cobs_decode(f) for k, f in sc.feed(d) if k == 'frame')
    except OSError:
      pass

  p = Peer(script)
  dev = uclog.connect(host=f'127.0.0.1:{p.port}', device_timeout=None)
  for i in range(200):
    dev.send(61, b'%d' % i)
  dev.close()
  p.t.join(3)
  assert [proto.mux_decode(f)[2] for f in got] == [b'%d' % i for i in range(200)]
