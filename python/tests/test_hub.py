# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Integration: the library against a real uclog-hub with the simulated `loopback:` device."""

import asyncio
import os
import pathlib
import subprocess
import sys
import time

import pytest

import uclog


def text_on(dev, port, want, timeout=5.0):
  """Port payloads until one contains `want`; returns all text seen."""
  seen = ''
  end = time.monotonic() + timeout
  while time.monotonic() < end:
    d = dev.recv(port, timeout=max(0.0, end - time.monotonic()))
    if d is None:
      break
    seen += d.decode(errors='replace')
    if want in seen:
      return seen
  raise AssertionError(f'{want!r} not received on port {port}; got {seen!r}')


def test_spawn_logs_terminal_semihost(hub_exe, sample_logdata):
  with uclog.connect(device='loopback:rate=20', logdata=sample_logdata,
                     stdin=lambda n: 'typed\r') as dev:
    info = dev.device_info
    assert info['board'] == 'loopback' and info['app_hash'] == bytes([0x5A] * 64)
    recs = []
    for r in dev.logs(timeout=2.0):
      recs.append(r)
      if len(recs) >= 10:
        break
    assert [r for r in recs if r.error] == []
    assert any(r.text == 'Hello World! loopback start' for r in recs)
    assert all(r.target == 1 for r in recs)
    assert str(recs[0]).count(':') >= 4

    dev.send(61, 'help\r')
    assert 'loopback commands' in text_on(dev, 61, '\n')
    dev.send(61, b'sh\r')
    assert 'sh: WRITE r=Some(20) errno=Some(0)' in text_on(dev, 61, 'WRITE')
    assert dev.console(timeout=2.0) == (1, b'hello from loopback\n')
    dev.send(61, 'read\r')
    assert 'read: n=6 errno=Some(0) "typed\\n"' in text_on(dev, 61, 'read:')
    dev.send(0, 'echo on port zero\r')
    assert dev.recv(0, timeout=2.0) == b'on port zero\r\n'
    dev.send(61, 'big\r')
    big = dev.recv(61, timeout=2.0)
    assert len(big) == uclog.proto.MAX_PORT_DATA
    assert dev.alive

    # One client at a time.
    with pytest.raises(uclog.HubBusyError):
      uclog.connect(host=dev.address)

    hubs = [h for h in uclog.list_hubs() if h.port == int(dev.address.rsplit(':', 1)[1])]
    assert len(hubs) == 1 and hubs[0].busy and hubs[0].device == 'loopback:rate=20'
    proc = dev._hub.proc
  # The hub started for the connection stops with it.
  assert proc.wait(5) is not None
  assert not [h for h in uclog.list_hubs() if h.pid == proc.pid]


def test_existing_hub_backlog_and_skip(hub_exe, sample_logdata):
  hub = uclog.start_hub('loopback:rate=50')
  try:
    host = f'127.0.0.1:{hub.port}'
    time.sleep(0.5)  # the device logs while nobody is connected
    with uclog.connect(host=host, logdata=sample_logdata) as dev:
      first = next(dev.logs(timeout=2.0))
      assert first.text == 'Hello World! loopback start'  # from the backlog
    time.sleep(0.3)
    with uclog.connect(host=host, logdata=sample_logdata, skip_backlog=True) as dev:
      recs = [r for _, r in zip(range(5), dev.logs(timeout=2.0))]
      assert len(recs) == 5 and all('loopback start' not in r.text for r in recs)
    # connect() by device finds the running hub instead of starting another.
    with uclog.connect(device='loopback:rate=50', logdata=sample_logdata) as dev:
      assert dev.address == host and dev._hub is None
  finally:
    assert uclog.stop_hub(hub.pid)
  assert hub.proc.wait(5) is not None


def test_no_logdata_error_records(hub_exe, tmp_path):
  with uclog.connect(device='loopback:rate=20', cache_dir=tmp_path) as dev:
    r = next(dev.logs(timeout=3.0))
    assert r.error == 'no_logdata' and r.level == 'ERROR' and 'target 1' in r.text


def test_logdata_from_cache(hub_exe, tmp_path, sample_logdata):
  (tmp_path / (bytes([0x5A] * 64).hex() + '.logdata')).write_bytes(pathlib.Path(sample_logdata).read_bytes())
  with uclog.connect(device='loopback:rate=20', cache_dir=tmp_path) as dev:
    r = next(dev.logs(timeout=3.0))
    assert r.error is None, r
    assert any(n.startswith('logdata: cache') for n in dev.notes)


def test_list_devices(hub_exe):
  devs = uclog.list_devices()
  assert isinstance(devs, list)
  assert all(d.uri and d.kind != 'hub' for d in devs)


def test_aio(hub_exe, sample_logdata):
  async def main():
    async with await uclog.aio.connect(device='loopback:rate=20', logdata=sample_logdata) as dev:
      info = await dev.device_info()
      assert info['board'] == 'loopback'
      await dev.send(61, 'echo async\r')
      got = b''
      while b'async' not in got:
        got += await dev.recv(61, timeout=2.0)
      n = 0
      async for r in dev.logs(timeout=2.0):
        n += 1
        if n == 3:
          break
      assert n == 3
  asyncio.run(main())


@pytest.mark.skipif(sys.platform == 'win32', reason='SIGKILL')
def test_killed_script_stops_its_hub(hub_exe):
  # The hub sees its stdin (and stderr) close and exits cleanly: no hub, no registry entry left.
  code = ('import os, sys, time, uclog\n'
          'd = uclog.connect(device="loopback:rate=20")\n'
          'print(d._hub.pid, flush=True)\n'
          'time.sleep(60)\n')
  env = dict(os.environ, PYTHONPATH=os.pathsep.join(sys.path))
  p = subprocess.Popen([sys.executable, '-c', code], stdout=subprocess.PIPE, env=env)
  try:
    pid = int(p.stdout.readline())
    assert [h for h in uclog.list_hubs() if h.pid == pid]
  finally:
    p.kill()
    p.wait()
    p.stdout.close()
  end = time.monotonic() + 5
  while uclog.hub._pid_alive(pid) and time.monotonic() < end:
    time.sleep(0.05)
  assert not uclog.hub._pid_alive(pid)
  assert not os.path.exists(os.path.join(uclog.hub.registry_dir(), f'{pid}.json'))
