# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Port-63 semihost requests from the device (protocol.md §5.5).

A connected script answers the device's syscalls (D3); the hub answers the time ops itself (D31).
By default file ops get ENOSYS. With `files_dir`, they work inside that directory only, like the
native tools (absolute paths, `..` escapes and symlinks leaving it fail with EACCES).
"""

import errno as host_errno
import os
import stat as stat_mod
import struct
import time

__all__ = ['Handler', 'ERRNO']

OPEN, WRITE, READ = 8, 9, 11
SYSTEM, RENAME, ISATTY, UNLINK, LINK, STAT, FSTAT, CLOSE = 0, 1, 2, 3, 4, 5, 6, 7
LSEEK, MKDIR, TIMEOFDAY = 10, 12, 13
SYS_CLOCK, SYS_TIME, SYS_ELAPSED, SYS_TICKFREQ = 0x10, 0x11, 0x30, 0x31

# newlib <sys/errno.h> (D25)
ERRNO = {
  'EPERM': 1, 'ENOENT': 2, 'EINTR': 4, 'EIO': 5, 'EBADF': 9, 'EAGAIN': 11, 'ENOMEM': 12,
  'EACCES': 13, 'EEXIST': 17, 'ENOTDIR': 20, 'EISDIR': 21, 'EINVAL': 22, 'EMFILE': 24,
  'ENOSPC': 28, 'ESPIPE': 29, 'EROFS': 30, 'ENOSYS': 88, 'ENOTEMPTY': 90, 'ENAMETOOLONG': 91,
}

# newlib open flags
O_ACCMODE, O_RDONLY, O_WRONLY, O_RDWR = 3, 0, 1, 2
O_APPEND, O_CREAT, O_TRUNC, O_EXCL = 0x8, 0x200, 0x400, 0x800

WRITE_NO_REPLY = 0x80000000
MAX_READ = 1024   # a READ reply fits in one frame (§2.4)
MAX_FILES = 64


def reply(r: int, err: int = 0) -> bytes:
  return struct.pack('<ii', r, err)


def error_reply(op: int, name: str) -> bytes:
  """`<i32 -1, i32 errno>`; FSTAT error replies keep the full 48 bytes (§5.5.4)."""
  r = reply(-1, ERRNO[name])
  return r + b'\0' * (40 if op == FSTAT else 0)


def map_error(e: OSError) -> str:
  """Host OSError → newlib errno name."""
  names = {
    host_errno.ENOENT: 'ENOENT', host_errno.EACCES: 'EACCES', host_errno.EPERM: 'EACCES',
    host_errno.EEXIST: 'EEXIST', host_errno.EINVAL: 'EINVAL', host_errno.EISDIR: 'EISDIR',
    host_errno.ENOTDIR: 'ENOTDIR', host_errno.ENOTEMPTY: 'ENOTEMPTY', host_errno.ENOSPC: 'ENOSPC',
    host_errno.EROFS: 'EROFS', host_errno.EINTR: 'EINTR', host_errno.EAGAIN: 'EAGAIN',
    host_errno.ENOMEM: 'ENOMEM', host_errno.ENAMETOOLONG: 'ENAMETOOLONG',
    host_errno.ESPIPE: 'ESPIPE', host_errno.EMFILE: 'EMFILE', host_errno.EBADF: 'EBADF',
  }
  return names.get(e.errno, 'EIO')


def time_reply(op: int):
  """Answer a time op (normally the hub does, D31)."""
  wall = time.time()
  ns = time.perf_counter_ns()  # §5.5.3: the Python counter
  if op == SYS_CLOCK:
    return struct.pack('<I', (ns // 10_000_000) & 0xFFFFFFFF)
  if op == SYS_TIME:
    return struct.pack('<q', int(wall // 1))
  if op == SYS_ELAPSED:
    return struct.pack('<Q', ns)
  if op == SYS_TICKFREQ:
    return struct.pack('<Q', 1_000_000_000)
  if op == TIMEOFDAY:
    sec = int(wall // 1)
    return struct.pack('<QI', sec, min(int((wall - sec) * 1e6 + 0.5), 999_999))
  return None


class Handler:
  """Answers port-63 requests.

  `handle(frame)` returns (reply or None, console) where console is (fid, data) for stdout/stderr
  writes, or ('stdin', max) when the device reads stdin (call `stdin_reply`).
  """

  def __init__(self, files_dir=None):
    self.root = os.path.realpath(os.fspath(files_dir)) if files_dir is not None else None
    self.files = {}
    self.next_fid = 3

  def close_all(self):
    for f in self.files.values():
      try:
        f.close()
      except OSError:
        pass
    self.files.clear()

  def handle(self, frame: bytes):
    if not frame:
      return error_reply(0xFF, 'EINVAL'), None
    op, a = frame[0], bytes(frame[1:])
    try:
      return self._handle(op, a)
    except (struct.error, UnicodeDecodeError, ValueError):
      return error_reply(op, 'EINVAL'), None
    except OSError as e:  # a host file error the op did not map itself
      return error_reply(op, map_error(e)), None

  def _handle(self, op, a):
    t = time_reply(op) if op in (TIMEOFDAY, SYS_CLOCK, SYS_TIME, SYS_ELAPSED, SYS_TICKFREQ) else None
    if t is not None:
      if a:
        raise ValueError('time ops take no arguments')
      return t, None
    if op == WRITE:
      raw, = struct.unpack_from('<I', a)
      fid, want_reply, data = raw & 0x7FFFFFFF, not raw & WRITE_NO_REPLY, a[4:]
      console = None
      if fid in (1, 2):
        r, console = reply(len(data)), (fid, data)
      elif fid in self.files:
        try:
          self.files[fid].write(data)
          r = reply(len(data))
        except OSError as e:
          r = error_reply(op, map_error(e))
      else:
        r = error_reply(op, 'EBADF')
      return (r if want_reply else None), console
    if op == READ:
      if len(a) != 8:
        raise ValueError('bad size')
      fid, n = struct.unpack('<iI', a)
      n = min(n, MAX_READ)
      if fid == 0:
        return None, ('stdin', n)
      f = self.files.get(fid)
      if f is None:
        return error_reply(op, 'EBADF'), None
      try:
        d = f.read(n)
      except OSError as e:
        return error_reply(op, map_error(e)), None
      return reply(len(d)) + d, None
    if op in (ISATTY, FSTAT, CLOSE):
      if len(a) != 4:
        raise ValueError('bad size')
      fid, = struct.unpack('<i', a)
      return self._fid_op(op, fid), None
    if op == OPEN:
      flags, = struct.unpack_from('<i', a)
      return self._open(flags, a[4:].decode('utf-8')), None
    if op == MKDIR:
      mode, = struct.unpack_from('<i', a)
      if self.root is None:
        return error_reply(op, 'ENOSYS'), None
      try:
        os.mkdir(self._resolve(a[4:].decode('utf-8')), mode & 0o7777)
        return reply(0), None
      except _Errno as e:
        return error_reply(op, e.name), None
      except OSError as e:
        return error_reply(op, map_error(e)), None
    return error_reply(op, 'ENOSYS'), None

  def stdin_reply(self, line: bytes, n: int) -> bytes:
    """READ reply for a stdin line (a trailing CR becomes LF)."""
    if line.endswith(b'\r'):
      line = line[:-1] + b'\n'
    line = line[:n]
    return reply(len(line)) + line

  def _fid_op(self, op, fid):
    if op == ISATTY:
      if fid in (0, 1, 2):
        return reply(1)
      return reply(0) if fid in self.files else error_reply(op, 'EBADF')
    if op == CLOSE:
      if fid in (0, 1, 2):
        return reply(0)
      f = self.files.pop(fid, None)
      if f is None:
        return error_reply(op, 'EBADF')
      try:
        f.close()
      except OSError as e:  # e.g. a deferred write error; the fid is gone either way
        return error_reply(op, map_error(e))
      return reply(0)
    # FSTAT
    if fid in (0, 1, 2):
      return _fstat(0o020666, 0, 1, 0, 0, 0, 0, 0, 0)
    f = self.files.get(fid)
    if f is None:
      return error_reply(op, 'EBADF')
    try:
      s = os.fstat(f.fileno())
    except OSError as e:
      return error_reply(op, map_error(e))
    kind = 0o040000 if stat_mod.S_ISDIR(s.st_mode) else 0o100000
    return _fstat(kind | (s.st_mode & 0o7777), s.st_ino & 0xFFFFFFFF, s.st_nlink, s.st_uid,
                  s.st_gid, min(s.st_size, 0xFFFFFFFF), int(s.st_atime), int(s.st_mtime),
                  int(s.st_ctime))

  def _open(self, flags, path):
    if self.root is None:
      return error_reply(OPEN, 'ENOSYS')
    try:
      p = self._resolve(path)
    except _Errno as e:
      return error_reply(OPEN, e.name)
    acc = flags & O_ACCMODE
    mode = {O_RDONLY: 'rb', O_WRONLY: 'wb', O_RDWR: 'r+b'}.get(acc, 'r+b')
    osflags = {O_RDONLY: os.O_RDONLY, O_WRONLY: os.O_WRONLY}.get(acc, os.O_RDWR)
    if flags & O_APPEND:
      osflags |= os.O_APPEND
      mode = 'ab' if acc == O_WRONLY else 'a+b'
    if flags & O_TRUNC:
      osflags |= os.O_TRUNC
    if flags & O_CREAT:
      osflags |= os.O_CREAT
      if flags & O_EXCL:
        osflags |= os.O_EXCL
    osflags |= getattr(os, 'O_BINARY', 0)
    if len(self.files) >= MAX_FILES:
      return error_reply(OPEN, 'EMFILE')
    try:
      fd = os.open(p, osflags, 0o666)
      f = os.fdopen(fd, mode, buffering=0)
    except OSError as e:
      return error_reply(OPEN, map_error(e))
    fid = self.next_fid
    self.next_fid = 3 if self.next_fid >= 0x7FFFFFF0 else self.next_fid + 1
    self.files[fid] = f
    return reply(fid)

  def _resolve(self, path):
    if not path:
      raise _Errno('ENOENT')
    if os.path.isabs(path) or path.startswith(('/', '\\')) or (len(path) > 1 and path[1] == ':'):
      raise _Errno('EACCES')
    parts = []
    for c in path.replace('\\', '/').split('/'):
      if c in ('', '.'):
        continue
      if c == '..':
        if not parts:
          raise _Errno('EACCES')
        parts.pop()
      else:
        parts.append(c)
    full = os.path.join(self.root, *parts)
    # Resolve symlinks in the existing part of the path to catch ones that leave the sandbox.
    real = os.path.realpath(full)
    if os.path.commonpath([real, self.root]) != self.root:
      raise _Errno('EACCES')
    return full


class _Errno(Exception):
  def __init__(self, name):
    super().__init__(name)
    self.name = name


def _fstat(mode, ino, nlink, uid, gid, size, atime, mtime, ctime) -> bytes:
  # <i32 0, i32 0> then mode, ino, dev(0), nlink, uid, gid, size, atime, mtime, ctime (§5.5.2)
  vals = (mode, ino, 0, nlink, uid, gid, size, atime, mtime, ctime)
  return reply(0) + struct.pack('<10I', *(v & 0xFFFFFFFF for v in vals))
