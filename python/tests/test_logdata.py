# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

import glob
import os
import pathlib
import struct

import pytest
from conftest import ROOT

from uclog import cbor
from uclog.logdata import LogData, LogDataError, _Int, elf_section, format_c


@pytest.mark.parametrize('fmt,vals,out', [
  ('%5d|%-5d|%05d', [42, 42, 42], '   42|42   |00042'),
  ('%+d % d', [5, 5], '+5  5'),
  ('%.3d|%05.3d|%-05.3d|', [7, 7, 7], '007|  007|007  |'),
  ('%x %X %u', [_Int(-1, 4), _Int(-1, 4), _Int(-1, 4)], 'ffffffff FFFFFFFF 4294967295'),
  ('%lx', [_Int(-1, 8)], 'ffffffffffffffff'),
  ('%#x %#o %#x', [255, 8, 0], '0xff 010 0'),
  ('%08x|%-8x|', [0xBEEF, 0xBEEF], '0000beef|beef    |'),
  ('%c%c', [65, 'B'], 'AB'),
  ('%.2s|%5s|%-5s|', ['abc', 'ab', 'ab'], 'ab|   ab|ab   |'),
  ('%8.3f %e %g', [3.14159, 12345.678, 0.0001], '   3.142 1.234568e+04 0.0001'),
  ('100%% %s', ['done'], '100% done'),
  ('%s %s', [_Int(3, 4), 1.5], '3 1.5'),
  ('%d', [2.9], '2'),
  ('%llu %zu', [_Int(2**63, 8), 5], '9223372036854775808 5'),
])
def test_format_c(fmt, vals, out):
  assert format_c(fmt, vals) == out


def test_format_c_arg_count():
  with pytest.raises(ValueError, match='more conversions'):
    format_c('%d %d', [1])
  with pytest.raises(ValueError, match='fewer conversions'):
    format_c('%d', [1, 2])


def elf32(sections):
  """A minimal little-endian ELF32 with the given (name, data) sections."""
  names = b'\0' + b''.join(n.encode() + b'\0' for n, _ in sections) + b'.shstrtab\0'
  body = b''
  offs = []
  for _, d in sections:
    offs.append(52 + len(body))
    body += d
  str_off = 52 + len(body)
  body += names
  shoff = 52 + len(body)
  hdr = b'\x7fELF\x01\x01\x01' + b'\0' * 9
  hdr += struct.pack('<HHIIIIIHHHHHH', 2, 40, 1, 0, 0, shoff, 0, 52, 0, 0, 40,
                     len(sections) + 2, len(sections) + 1)
  sh = b'\0' * 40
  pos = 1
  for (n, d), o in zip(sections, offs):
    sh += struct.pack('<10I', pos, 1, 0, 0, o, len(d), 0, 0, 1, 0)
    pos += len(n) + 1
  sh += struct.pack('<10I', pos, 3, 0, 0, str_off, len(names), 0, 0, 1, 0)
  return hdr + body + sh


def test_elf_reader(tmp_path, sample_logdata):
  data = pathlib.Path(sample_logdata).read_bytes()
  elf = elf32([('.text', b'\x01\x02'), ('.logdata_cbor', data)])
  assert elf_section(elf, '.logdata_cbor') == data
  assert elf_section(elf, '.nothere') is None
  p = tmp_path / 'app.elf'
  p.write_bytes(elf)
  ld = LogData.load(p)
  assert ld.target == LogData.load(sample_logdata).target
  p.write_bytes(elf32([('.text', b'x')]))
  with pytest.raises(LogDataError, match='no .logdata_cbor'):
    LogData.load(p)


def test_real_firmware_elf():
  # The NUCLEO test firmware, when it has been built.
  elfs = glob.glob(os.path.join(ROOT, 'firmware', '*', 'build', '*.elf'))
  if not elfs:
    pytest.skip('firmware not built')
  ld = LogData.load(elfs[0])
  assert ld.fmts


def test_hash_from_file_name(tmp_path, sample_logdata):
  h = bytes(range(64))
  p = tmp_path / f'{h.hex()}.logdata'
  p.write_bytes(pathlib.Path(sample_logdata).read_bytes())
  assert LogData.load(p).app_hash == h
  assert LogData.load(sample_logdata).app_hash is None


def test_not_logdata(tmp_path):
  p = tmp_path / 'x.logdata'
  p.write_bytes(cbor.dumps({'hello': 1}))
  with pytest.raises(LogDataError):
    LogData.load(p)


def test_auth_hint():
  from uclog.resolver import auth_hint
  assert auth_hint(401, False).startswith('No token was sent')
  assert auth_hint(401, True).startswith('The token was not accepted')
  assert 'logdata:read' in auth_hint(403, True)
  assert auth_hint(500, True) is None


def _service(public):
  """A logdata service that refuses any token and, when `public`, allows reading without one."""
  import http.server
  import threading
  body = pathlib.Path(ROOT, 'spec', 'vectors', 'sample.logdata').read_bytes()

  class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
      ok = public and 'Authorization' not in self.headers
      self.send_response(200 if ok else 401)
      self.send_header('Content-Length', str(len(body) if ok else 0))
      self.end_headers()
      if ok:
        self.wfile.write(body)

    def log_message(self, *a):
      pass

  srv = http.server.HTTPServer(('127.0.0.1', 0), H)
  threading.Thread(target=srv.serve_forever, daemon=True).start()
  return srv, f'http://127.0.0.1:{srv.server_port}'


def test_refused_token_retries_without_it(tmp_path):
  from uclog.resolver import Resolver
  srv, url = _service(public=True)
  try:
    r = Resolver(cache_dir=tmp_path, domain='example.com', url=url, token='stale')
    lds, source, notes = r.resolve(bytes(64))
    assert lds and source.startswith('remote') and r.token_refused
    assert 'token was refused' in notes[0]
  finally:
    srv.shutdown()
  srv, url = _service(public=False)
  try:
    r = Resolver(cache_dir=tmp_path / 'b', domain='example.com', url=url, token='stale')
    lds, source, notes = r.resolve(bytes(64))
    assert not lds and not r.token_refused
    assert 'HTTP 401' in notes[0] and 'The token was not accepted' in notes[0]
  finally:
    srv.shutdown()
