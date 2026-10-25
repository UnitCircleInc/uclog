# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

import os
import struct
import sys

import pytest

from uclog import semihost as sh
from uclog.semihost import ERRNO, Handler


def req(op, *parts):
  return bytes([op]) + b''.join(parts)


def i32(v):
  return struct.pack('<i', v)


def u32(v):
  return struct.pack('<I', v)


def rv(r):
  return struct.unpack_from('<ii', r)


def test_no_files_dir_means_enosys():
  h = Handler()
  r, _ = h.handle(req(sh.OPEN, i32(0), b'a.txt'))
  assert rv(r) == (-1, ERRNO['ENOSYS'])
  r, _ = h.handle(req(sh.MKDIR, i32(0o755), b'd'))
  assert rv(r) == (-1, ERRNO['ENOSYS'])
  for op in (sh.SYSTEM, sh.RENAME, sh.UNLINK, sh.LINK, sh.STAT, sh.LSEEK, 0x55):
    r, _ = h.handle(req(op, b'xx'))
    assert rv(r) == (-1, ERRNO['ENOSYS'])


def test_stdio():
  h = Handler()
  r, c = h.handle(req(sh.WRITE, u32(1), b'hello'))
  assert rv(r) == (5, 0) and c == (1, b'hello')
  r, c = h.handle(req(sh.WRITE, u32(2 | sh.WRITE_NO_REPLY), b'err'))
  assert r is None and c == (2, b'err')
  r, c = h.handle(req(sh.READ, i32(0), u32(5000)))
  assert r is None and c == ('stdin', sh.MAX_READ)
  assert h.stdin_reply(b'abc\r', 10) == struct.pack('<ii', 4, 0) + b'abc\n'
  assert rv(h.handle(req(sh.ISATTY, i32(1)))[0]) == (1, 0)
  r, _ = h.handle(req(sh.FSTAT, i32(1)))
  assert len(r) == 48 and struct.unpack_from('<I', r, 8)[0] == 0o020666


def test_errors():
  h = Handler()
  r, _ = h.handle(req(sh.FSTAT, i32(9)))
  assert len(r) == 48 and rv(r) == (-1, ERRNO['EBADF'])
  r, _ = h.handle(req(sh.CLOSE, b'\x01'))  # malformed
  assert rv(r) == (-1, ERRNO['EINVAL'])
  r, _ = h.handle(req(sh.WRITE, u32(7), b'x'))
  assert rv(r) == (-1, ERRNO['EBADF'])


def test_file_round_trip(tmp_path):
  h = Handler(tmp_path)
  flags = sh.O_WRONLY | sh.O_CREAT | sh.O_TRUNC
  fid, err = rv(h.handle(req(sh.OPEN, i32(flags), b'sub/../out.bin'))[0])
  assert err == 0 and fid >= 3
  assert rv(h.handle(req(sh.WRITE, u32(fid), b'0123456789'))[0]) == (10, 0)
  r, _ = h.handle(req(sh.FSTAT, i32(fid)))
  assert len(r) == 48 and struct.unpack_from('<I', r, 8 + 24)[0] == 10
  assert rv(h.handle(req(sh.CLOSE, i32(fid)))[0]) == (0, 0)
  assert (tmp_path / 'out.bin').read_bytes() == b'0123456789'
  fid, _ = rv(h.handle(req(sh.OPEN, i32(sh.O_RDONLY), b'out.bin'))[0])
  r, _ = h.handle(req(sh.READ, i32(fid), u32(4)))
  assert rv(r) == (4, 0) and r[8:] == b'0123'
  r, _ = h.handle(req(sh.READ, i32(fid), u32(100)))
  assert r[8:] == b'456789'
  r, _ = h.handle(req(sh.READ, i32(fid), u32(100)))
  assert rv(r) == (0, 0)
  assert rv(h.handle(req(sh.MKDIR, i32(0o755), b'dir'))[0]) == (0, 0)
  assert (tmp_path / 'dir').is_dir()
  r, _ = h.handle(req(sh.OPEN, i32(sh.O_WRONLY | sh.O_CREAT | sh.O_EXCL), b'out.bin'))
  assert rv(r) == (-1, ERRNO['EEXIST'])
  r, _ = h.handle(req(sh.OPEN, i32(sh.O_RDONLY), b'missing'))
  assert rv(r) == (-1, ERRNO['ENOENT'])
  h.close_all()  # files the device left open


@pytest.mark.parametrize('path', [b'../x', b'/etc/passwd', b'a/../../x', b''])
def test_sandbox_escapes(tmp_path, path):
  h = Handler(tmp_path / 'root')
  os.makedirs(tmp_path / 'root')
  r, _ = h.handle(req(sh.OPEN, i32(sh.O_WRONLY | sh.O_CREAT), path))
  assert rv(r)[0] == -1 and rv(r)[1] in (ERRNO['EACCES'], ERRNO['ENOENT'])


@pytest.mark.skipif(sys.platform == 'win32', reason='symlinks need privileges on Windows')
def test_symlink_escape(tmp_path):
  root = tmp_path / 'root'
  root.mkdir()
  (tmp_path / 'secret').write_text('x')
  os.symlink(tmp_path / 'secret', root / 'link')
  h = Handler(root)
  r, _ = h.handle(req(sh.OPEN, i32(sh.O_RDONLY), b'link'))
  assert rv(r) == (-1, ERRNO['EACCES'])


def test_close_error_is_a_reply(tmp_path):
  class Failing:
    def close(self):
      raise OSError(5, 'deferred write error')

  h = Handler(tmp_path)
  h.files[7] = Failing()
  r, _ = h.handle(req(sh.CLOSE, i32(7)))
  assert rv(r) == (-1, ERRNO['EIO'])
  assert 7 not in h.files


def test_time_ops_use_perf_counter(monkeypatch):
  # protocol.md §5.5.3: Python hosts use time.perf_counter_ns().
  monkeypatch.setattr(sh.time, 'perf_counter_ns', lambda: 1_234_000_000_000)
  monkeypatch.setattr(sh.time, 'monotonic_ns', lambda: 1)
  assert sh.time_reply(sh.SYS_ELAPSED) == struct.pack('<Q', 1_234_000_000_000)
  assert sh.time_reply(sh.SYS_CLOCK) == struct.pack('<I', 123_400)
