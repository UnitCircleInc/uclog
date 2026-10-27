# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""The examples (python/examples/), run as a user would run them, against a real uclog-hub with
the simulated `loopback:` device."""

import json
import os
import socket
import subprocess
import sys
import time

import uclog

from conftest import HERE

EXAMPLES = os.path.join(HERE, '..', 'examples')
SRC = os.path.join(HERE, '..', 'src')


def run(*args, stdin=None, timeout=60):
  env = {**os.environ, 'PYTHONPATH': SRC + os.pathsep + os.environ.get('PYTHONPATH', '')}
  r = subprocess.run([sys.executable, *args], input=stdin, capture_output=True, text=True,
                     timeout=timeout, env=env)
  assert r.returncode == 0, f'{args}: exit {r.returncode}\n{r.stdout}\n{r.stderr}'
  return r.stdout


def example(name):
  return os.path.join(EXAMPLES, name)


def test_capture_logs_filters(hub_exe, sample_logdata):
  out = run(example('capture_logs.py'), '-d', 'loopback:rate=20', '-l', sample_logdata,
            '--seconds', '2', '--level', 'WARN', '--grep', r'temp=\d+')
  lines = out.splitlines()
  assert lines and all(':WARN:' in line and 'temp=' in line for line in lines), out


def test_command(hub_exe):
  out = run(example('command.py'), '-d', 'loopback:rate=0', 'echo', '{"value": 42}')
  assert json.loads(out) == {'rsp': 'echo', 'value': 42}
  info = json.loads(run(example('command.py'), '-d', 'loopback:rate=0', 'get-info'))
  assert info['board'] == 'loopback'


def test_terminal_piped(hub_exe):
  out = run(example('terminal.py'), '-d', 'loopback:rate=0', stdin='echo piped\nhelp\n')
  assert 'piped' in out and 'loopback commands' in out


def test_decode_capture(hub_exe, sample_logdata, tmp_path):
  # A capture: the raw bytes a raw-TCP client of a hub receives are the device's own.
  hub = uclog.start_hub('loopback:rate=50')
  try:
    with socket.create_connection(('127.0.0.1', hub.port), timeout=5) as s:
      s.settimeout(0.2)
      data, end = b'', time.monotonic() + 1.5
      while time.monotonic() < end:
        try:
          data += s.recv(65536)
        except socket.timeout:
          pass
  finally:
    uclog.stop_hub(hub.pid)
  cap = tmp_path / 'capture.bin'
  cap.write_bytes(data)
  out = run(example('decode_capture.py'), '-l', sample_logdata, str(cap))
  assert 'INFO:src/main.c:10:Hello World! loopback' in out, out
  assert not any(line.startswith('<') for line in out.splitlines()), out  # nothing undecodable


def test_pytest_fixture(hub_exe, sample_logdata):
  out = run('-m', 'pytest', os.path.join(EXAMPLES, 'pytest'), '-q', '-p', 'no:cacheprovider',
            '--uclog-device', 'loopback:rate=20', '--uclog-logdata', sample_logdata, timeout=120)
  assert '4 passed' in out, out
