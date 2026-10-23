# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Finding hubs, listing devices, and starting and stopping hubs (plan 06 §4).

The library never opens a device itself: it talks to the native hub (`uclog-hub`, or `uclog hub`).
"""

import atexit
import json
import logging
import os
import shutil
import signal
import subprocess
import sys
import threading
from dataclasses import dataclass, field
from typing import List, Optional

from .errors import HubStartError, NoHubError

__all__ = ['HubInfo', 'DeviceInfo', 'list_hubs', 'list_devices', 'start_hub', 'stop_hub',
           'find_executable', 'registry_dir', 'DEFAULT_PORT']

log = logging.getLogger('uclog.hub')

DEFAULT_PORT = 9000


@dataclass
class HubInfo:
  """A running hub."""
  name: str
  host: str
  port: int
  device: str = ''
  serial_number: str = ''
  kind: str = ''
  busy: Optional[bool] = None
  pid: Optional[int] = None
  local: bool = True
  description: str = ''

  @property
  def address(self) -> str:
    return f'{self.host}:{self.port}'

  def __str__(self):
    busy = ' [busy]' if self.busy else ''
    where = 'local' if self.local else 'mDNS'
    return f'{self.name} {self.address} {self.device or self.kind} {self.serial_number} ({where}){busy}'


@dataclass
class DeviceInfo:
  """A device the hub can open (`uclog-hub list --json`)."""
  kind: str
  uri: str
  name: str = ''
  serial: str = ''
  description: str = ''
  extra: dict = field(default_factory=dict, repr=False)

  def __str__(self):
    return f'{self.uri} {self.name} {self.serial} {self.description}'.rstrip()


# ---------------------------------------------------------------------------
# Executable
# ---------------------------------------------------------------------------

def _exe(name):
  return name + ('.exe' if sys.platform == 'win32' else '')


def find_executable() -> Optional[str]:
  """The hub executable: $UCLOG_HUB, the one bundled in the wheel, then `uclog-hub` or `uclog`
  on PATH."""
  env = os.environ.get('UCLOG_HUB')
  if env:
    return env
  bundled = os.path.join(os.path.dirname(__file__), 'bin', _exe('uclog-hub'))
  if os.path.isfile(bundled):
    return bundled
  return shutil.which('uclog-hub') or shutil.which('uclog')


def _is_uclog(exe):
  """`uclog` needs the `hub` subcommand; `uclog-hub` does not."""
  base = os.path.basename(exe).lower()
  return not base.startswith('uclog-hub')


def _require_executable():
  exe = find_executable()
  if exe is None:
    raise NoHubError('no hub running and no uclog-hub executable found: install a platform wheel, '
                     'put uclog-hub (or uclog) on PATH, or set UCLOG_HUB')
  return exe


# ---------------------------------------------------------------------------
# Registry and mDNS
# ---------------------------------------------------------------------------

def registry_dir() -> str:
  """Where hubs on this machine register (hub plan §7.1)."""
  if sys.platform.startswith('linux') and os.environ.get('XDG_RUNTIME_DIR'):
    return os.path.join(os.environ['XDG_RUNTIME_DIR'], 'uclog', 'hubs')
  if sys.platform == 'win32':
    local = os.environ.get('LOCALAPPDATA') or os.path.expanduser('~\\AppData\\Local')
    return os.path.join(local, 'uclog', 'run', 'hubs')
  if sys.platform == 'darwin':
    return os.path.expanduser('~/Library/Caches/ca.unitcircle.uclog/run/hubs')
  cache = os.environ.get('XDG_CACHE_HOME') or os.path.expanduser('~/.cache')
  return os.path.join(cache, 'uclog', 'run', 'hubs')


def _pid_alive(pid):
  if not pid:
    return False
  if sys.platform == 'win32':
    import ctypes
    k = ctypes.windll.kernel32
    h = k.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not h:
      return False
    code = ctypes.c_ulong()
    ok = k.GetExitCodeProcess(h, ctypes.byref(code))
    k.CloseHandle(h)
    return bool(ok) and code.value == 259  # STILL_ACTIVE
  try:
    os.kill(pid, 0)
  except PermissionError:
    return True
  except OSError:
    return False
  return True


def _local_hubs():
  d = registry_dir()
  try:
    names = os.listdir(d)
  except OSError:
    return []
  out = []
  for n in sorted(names):
    if not n.endswith('.json'):
      continue
    try:
      with open(os.path.join(d, n), encoding='utf-8') as f:
        j = json.load(f)
    except (OSError, ValueError):
      continue
    if not _pid_alive(j.get('pid')):
      continue
    out.append(HubInfo(name=j.get('name', ''), host='127.0.0.1', port=j.get('port', DEFAULT_PORT),
                       device=j.get('device', ''), serial_number=j.get('serial_number', ''),
                       kind=j.get('kind', ''), busy=j.get('busy'), pid=j.get('pid'), local=True))
  return out


def _mdns_hubs(timeout):
  try:
    from zeroconf import ServiceBrowser, Zeroconf
  except ImportError:
    log.info('mDNS discovery needs the zeroconf package (pip install "uclog[mdns]")')
    return []
  import time
  zc = Zeroconf()
  found = {}

  class L:
    def add_service(self, zc, type_, name):
      found[name] = None

    def update_service(self, zc, type_, name):
      found[name] = None

    def remove_service(self, zc, type_, name):
      found.pop(name, None)

  try:
    ServiceBrowser(zc, '_uclog._tcp.local.', L())
    time.sleep(timeout)
    out = []
    for name in list(found):
      info = zc.get_service_info('_uclog._tcp.local.', name, timeout=500)
      if info is None or not info.parsed_addresses():
        continue
      txt = {k.decode(errors='replace'): (v or b'').decode(errors='replace')
             for k, v in (info.properties or {}).items()}
      out.append(HubInfo(name=name.split('._uclog')[0], host=info.parsed_addresses()[0],
                         port=info.port, serial_number=txt.get('serial_number', ''),
                         kind=txt.get('kind', ''), description=txt.get('description', ''),
                         busy=txt.get('busy') in ('1', 'true'), local=False))
    return out
  finally:
    zc.close()


def list_hubs(discover_remote: bool = False, timeout: float = 1.0) -> List[HubInfo]:
  """Hubs on this machine (registry), plus remote hubs over mDNS when `discover_remote` is set
  (needs the `uclog[mdns]` extra)."""
  hubs = _local_hubs()
  if discover_remote:
    names = {h.name for h in hubs}
    hubs += [h for h in _mdns_hubs(timeout) if h.name not in names]
  return hubs


def list_devices(usb_filters=()) -> List[DeviceInfo]:
  """Local USB devices the hub can open (`uclog-hub list --json`)."""
  exe = _require_executable()
  args = [exe, 'list', '--json'] + (['--mdns', '0'] if _is_uclog(exe) else [])
  for f in usb_filters:
    args += ['--usb-filter', f]
  r = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
  if r.returncode != 0:
    raise NoHubError(f'{" ".join(args)}: {r.stderr.decode(errors="replace").strip()}')
  out = []
  for d in json.loads(r.stdout.decode() or '[]'):
    if d.get('kind') == 'hub':
      continue
    out.append(DeviceInfo(kind=d.get('kind', ''), uri=d.get('uri', ''), name=d.get('name', ''),
                          serial=d.get('serial', ''), description=d.get('description', ''), extra=d))
  return out


# ---------------------------------------------------------------------------
# Hubs started by the library (D30)
# ---------------------------------------------------------------------------

class SpawnedHub:
  """A hub process started by the library. It exits when `stop()` closes its stdin, or when this
  process dies (`--exit-on-stdin-eof`)."""

  def __init__(self, proc, ready, stderr_thread=None):
    self.proc = proc
    self._stderr_thread = stderr_thread
    self.port = ready['port']
    self.name = ready.get('name', '')
    self.device = ready.get('device', '')

  @property
  def pid(self):
    return self.proc.pid

  def stop(self, timeout=3.0):
    if self.proc.poll() is None:
      try:
        self.proc.stdin.close()
      except OSError:
        pass
      try:
        self.proc.wait(timeout)
      except subprocess.TimeoutExpired:
        self.proc.kill()
        self.proc.wait()
    for f in (self.proc.stdin, self.proc.stdout):
      try:
        f.close()
      except OSError:
        pass
    if self._stderr_thread is not None:
      self._stderr_thread.join(1)  # it closes stderr at EOF


_spawned: List[SpawnedHub] = []
_spawned_lock = threading.Lock()


@atexit.register
def _stop_all():
  with _spawned_lock:
    hubs = list(_spawned)
    _spawned.clear()
  for h in hubs:
    h.stop()


def _forward_stderr(stream, lines):
  with stream:
    for raw in iter(stream.readline, b''):
      line = raw.decode(errors='replace').rstrip()
      lines.append(line)
      del lines[:-20]
      log.info('%s', line)


def start_hub(device: str, extra_args=(), timeout: float = 15.0) -> SpawnedHub:
  """Start a hub for `device` (a URI such as `serial:/dev/ttyACM0`) and wait until it listens.
  It is stopped when the script exits."""
  exe = _require_executable()
  args = [exe] + (['hub'] if _is_uclog(exe) else [])
  args += ['-d', device, '--json-ready', '--exit-on-stdin-eof'] + list(extra_args)
  log.info('starting %s', ' '.join(args))
  kw = {}
  if sys.platform == 'win32':
    kw['creationflags'] = getattr(subprocess, 'CREATE_NO_WINDOW', 0)
  proc = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, **kw)
  errors = []
  err_thread = threading.Thread(target=_forward_stderr, args=(proc.stderr, errors), daemon=True)
  err_thread.start()
  result = []
  t = threading.Thread(target=lambda: result.append(proc.stdout.readline()), daemon=True)
  t.start()
  t.join(timeout)
  line = result[0] if result else b''
  try:
    ready = json.loads(line.decode())
  except ValueError:
    ready = None
  if not ready or 'port' not in ready:
    if proc.poll() is None:
      proc.kill()
    proc.wait()
    t.join(1)
    err_thread.join(1)
    proc.stdin.close()
    proc.stdout.close()
    detail = '; '.join(errors[-5:]) or f'exit code {proc.returncode}'
    raise HubStartError(f'hub for {device} did not start: {detail}')
  hub = SpawnedHub(proc, ready, err_thread)
  with _spawned_lock:
    _spawned.append(hub)
  return hub


def _forget(hub):
  with _spawned_lock:
    if hub in _spawned:
      _spawned.remove(hub)


def stop_hub(host_or_pid) -> bool:
  """Stop a hub on this machine: by pid (an int), or by port ("9001", "localhost:9001").
  Returns True if one was found."""
  if isinstance(host_or_pid, int):
    pid = host_or_pid
  else:
    tail = str(host_or_pid).rsplit(':', 1)[-1]
    port = int(tail) if tail.isdigit() else DEFAULT_PORT
    pid = next((h.pid for h in _local_hubs() if h.port == port), None)
    if pid is None:
      return False
  with _spawned_lock:
    mine = [h for h in _spawned if h.pid == pid]
  if mine:
    mine[0].stop()
    _forget(mine[0])
    return True
  if not _pid_alive(pid):
    return False
  os.kill(pid, signal.SIGTERM)
  return True
