# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""A connection to a hub (plan 06 §3–§5)."""

import collections
import ipaddress
import logging
import os
import socket
import threading
import time
from typing import Iterator, Optional

from . import cbor, proto
from .errors import (ApprovalTimeoutError, ConnectionClosedError, DeviceNotRespondingError,
                     HubBusyError, MultipleHubsError, NoHubError)
from .hub import DEFAULT_PORT, list_devices, list_hubs, start_hub
from .logdata import DecodeError, LogData, LogRecord
from .resolver import Resolver
from .semihost import Handler

__all__ = ['Connection', 'connect']

log = logging.getLogger('uclog')

PULSE_INTERVAL = 0.5
LIVENESS_TIMEOUT = 2.0
WAKE_GAP = 0.05


def _hex(b, n=32):
  s = bytes(b[:n]).hex()
  return s + ('…' if len(b) > n else '')


def parse_host(host: str):
  """"host", "host:port", "[v6]:port" or "tcp://host:port" → (host, port)."""
  h = host.strip()
  for scheme in ('tcp://', 'uclog://'):
    if h.startswith(scheme):
      h = h[len(scheme):]
  h = h.rstrip('/')
  if h.startswith('['):
    addr, _, rest = h[1:].partition(']')
    return addr, int(rest[1:]) if rest.startswith(':') else DEFAULT_PORT
  if h.count(':') == 1:
    addr, port = h.split(':')
    return addr, int(port)
  return h, DEFAULT_PORT


class Connection:
  """One client connection to a hub. Create it with `uclog.connect()`.

  A reader thread receives frames, answers port-63 syscalls and decodes logs; a writer thread
  sends frames and a pulse every 0.5 s. Received data waits in bounded queues: per port
  (`recv`), all frames (`frames`), log records (`logs`) and semihost console output (`console`).
  When a queue is full the oldest entry is dropped and counted in `dropped`.
  """

  def __init__(self, sock: socket.socket, *, resolver: Resolver = None, files_dir=None,
               stdin=None, max_queue: int = 10000, skip_backlog: bool = False, hub=None,
               address='', manual=None):
    self.address = address
    self._sock = sock
    self._resolver = resolver or Resolver()
    self._semihost = Handler(files_dir)
    self._stdin = stdin
    self._max = max_queue
    self._skip = skip_backlog
    self._hub = hub
    self._start = time.monotonic()
    self._cv = threading.Condition(threading.RLock())
    self._ports = collections.defaultdict(collections.deque)
    self._frames = collections.deque()
    self._logs = collections.deque()
    self._console = collections.deque()
    self._seq = 0
    #: Entries dropped from full queues, by port number or 'frames', 'logs', 'console'.
    self.dropped = collections.Counter()
    #: Resolver notes and other messages (also logged to the `uclog` logger).
    self.notes = []
    self._info = None
    self._app_hash = None
    self._decoders = {}
    for ld in (self._resolver.manual() if manual is None else manual):
      self._decoders[ld.target] = ld
    self._pending = None
    self._live = not skip_backlog
    self._first_rx = None
    self._last_rx = None
    self._closed = False
    self._out = collections.deque()
    self._out_cv = threading.Condition()
    self._scanner = proto.FrameScanner()
    self._zeros = 0          # length of the current run of 00 bytes
    self._stream_start = True
    self._reader = threading.Thread(target=self._read_loop, name='uclog-reader', daemon=True)
    self._writer = threading.Thread(target=self._write_loop, name='uclog-writer', daemon=True)
    self._reader.start()
    self._writer.start()

  # ------------------------------------------------------------------ public API

  def __enter__(self):
    return self

  def __exit__(self, *exc):
    self.close()
    return False

  def __repr__(self):
    return f'<uclog.Connection {self.address}{" closed" if self._closed else ""}>'

  @property
  def closed(self) -> bool:
    return self._closed

  @property
  def alive(self) -> bool:
    """Bytes (data or pulses) arrived within the last 2 s."""
    return self._last_rx is not None and time.monotonic() - self._last_rx <= LIVENESS_TIMEOUT

  @property
  def device_info(self) -> Optional[dict]:
    """The port-62 device info (board, app_hash, ...). Waits up to 2 s for it."""
    return self.wait_device_info(2.0)

  def wait_device_info(self, timeout: Optional[float] = 2.0) -> Optional[dict]:
    with self._cv:
      self._cv.wait_for(lambda: self._info is not None or self._closed, timeout)
      return self._info

  @property
  def logdata(self) -> dict:
    """The logdata in use, by target digit."""
    with self._cv:
      return dict(self._decoders)

  def load_logdata(self, path) -> LogData:
    """Use a logdata file (or ELF) from now on, for its target."""
    ld = LogData.load(path)
    with self._cv:
      self._decoders[ld.target] = ld
    return ld

  def send(self, port: int, data) -> None:
    """Send `data` (bytes, or str as UTF-8) on `port`."""
    if isinstance(data, str):
      data = data.encode('utf-8')
    if len(data) > proto.MAX_PORT_DATA:
      raise ValueError(f'{len(data)} bytes exceed the {proto.MAX_PORT_DATA}-byte port limit')
    self._queue_out(proto.wire_port(port, data))

  def send_frame(self, payload: bytes) -> None:
    """Send a complete payload (frame header included)."""
    if len(payload) > proto.MAX_PAYLOAD:
      raise ValueError(f'{len(payload)} bytes exceed the {proto.MAX_PAYLOAD}-byte frame limit')
    self._queue_out(proto.wire(payload))

  def recv(self, port: int, timeout: Optional[float] = None) -> Optional[bytes]:
    """The next payload received on `port`, or None after `timeout` seconds."""
    with self._cv:
      q = self._ports[port]
      self._wait(lambda: q, timeout)
      return q.popleft()[1] if q else None

  def frames(self, timeout: Optional[float] = None, duration: Optional[float] = None) -> Iterator[bytes]:
    """Every received payload (frame header included, pulses excluded), until `timeout` seconds
    pass without one, or `duration` seconds in all."""
    return self._drain(self._frames, timeout, duration)

  def logs(self, timeout: Optional[float] = None, duration: Optional[float] = None) -> Iterator[LogRecord]:
    """Decoded log records (and error records), until `timeout` seconds pass without one, or
    `duration` seconds in all."""
    return self._drain(self._logs, timeout, duration)

  def _drain(self, q, timeout, duration):
    end = None if duration is None else time.monotonic() + duration
    while True:
      t = timeout
      if end is not None:
        left = max(0.0, end - time.monotonic())
        t = left if t is None else min(t, left)
      with self._cv:
        self._wait(lambda: q, t)
        if not q or (end is not None and time.monotonic() >= end):
          return
        item = q.popleft()
      yield item

  def console(self, timeout: Optional[float] = None):
    """Semihost stdout/stderr output: (fid, bytes), or None after `timeout` seconds. Writes to
    the same stream that have not been read yet are joined."""
    with self._cv:
      self._wait(lambda: self._console, timeout)
      return self._console.popleft() if self._console else None

  def command(self, obj: dict, timeout: float = 2.0):
    """Send a CBOR command map on port 0 and wait for the reply whose `rsp` equals its `cmd`.
    Other port-0 frames stay queued for `recv(0)`. Returns the reply map, or None on timeout."""
    if 'cmd' not in obj:
      raise ValueError("a command map needs a 'cmd' key")
    with self._cv:
      after = self._seq
      q = self._ports[proto.PORT_COMMAND]
    self.send(proto.PORT_COMMAND, cbor.dumps(obj))

    def find():
      for i, (seq, data) in enumerate(q):
        if seq <= after:
          continue
        try:
          m = cbor.loads(data)
        except cbor.CborError:
          continue
        if isinstance(m, dict) and m.get('rsp') == obj['cmd']:
          return i, m
      return None

    with self._cv:
      found = None
      deadline = time.monotonic() + timeout
      while True:
        found = find()
        if found or self._closed:
          break
        left = deadline - time.monotonic()
        if left <= 0:
          break
        self._cv.wait(left)
      if not found:
        if self._closed:
          raise ConnectionClosedError(f'{self.address}: connection closed')
        return None
      del q[found[0]]
      return found[1]

  def close(self) -> None:
    """Close the connection. A hub started for it by the library is stopped."""
    with self._cv:
      self._closed = True
      self._cv.notify_all()
    with self._out_cv:
      self._out_cv.notify_all()
    try:
      self._sock.shutdown(socket.SHUT_RDWR)
    except OSError:
      pass
    self._sock.close()
    for t in (self._reader, self._writer):
      if t is not threading.current_thread():
        t.join(2)
    self._semihost.close_all()
    if self._hub is not None:
      from .hub import _forget
      self._hub.stop()
      _forget(self._hub)
      self._hub = None

  # ------------------------------------------------------------------ internals

  def _wait(self, pred, timeout):
    """Wait (holding the lock) until pred() or timeout; raise if closed with nothing left."""
    self._cv.wait_for(lambda: pred() or self._closed, timeout)
    if not pred() and self._closed:
      raise ConnectionClosedError(f'{self.address}: connection closed')

  def _push(self, q, item, key):
    if len(q) >= self._max:
      q.popleft()
      self.dropped[key] += 1
    q.append(item)

  def _queue_out(self, b):
    if self._closed:
      raise ConnectionClosedError(f'{self.address}: connection closed')
    self._enqueue(b)

  def _enqueue(self, b):
    with self._out_cv:
      self._out.append(b)
      self._out_cv.notify()

  def _write_loop(self):
    last_tx = None
    try:
      self._sock.sendall(proto.PULSE)  # wake (protocol.md §2.3)
      last_tx = time.monotonic()
      ready = last_tx + WAKE_GAP
      while not self._closed:
        with self._out_cv:
          if not self._out:
            self._out_cv.wait(max(0.0, last_tx + PULSE_INTERVAL - time.monotonic()))
          batch = list(self._out)
          self._out.clear()
        now = time.monotonic()
        if batch:
          if now - last_tx > LIVENESS_TIMEOUT:
            self._sock.sendall(proto.PULSE)
            ready = time.monotonic() + WAKE_GAP
          if time.monotonic() < ready:
            time.sleep(ready - time.monotonic())
          self._sock.sendall(b''.join(batch))
          last_tx = time.monotonic()
        elif now - last_tx >= PULSE_INTERVAL:
          self._sock.sendall(proto.PULSE)
          last_tx = now
    except OSError:
      pass  # the reader sees the connection end and marks it closed once all data is read

  def _mark_closed(self):
    with self._cv:
      self._closed = True
      self._cv.notify_all()
    with self._out_cv:
      self._out_cv.notify_all()

  def _read_loop(self):
    try:
      while True:
        d = self._sock.recv(65536)
        if not d:
          break
        self._on_bytes(d)
    except OSError:
      pass
    except Exception:  # a bug here must not leave the script waiting forever
      log.exception('uclog reader')
    finally:
      self._mark_closed()

  def _on_bytes(self, d):
    now = time.monotonic()
    with self._cv:
      if self._first_rx is None:
        self._first_rx = now
      self._last_rx = now
      pos = None if self._live else self._find_pulse(d)
      if pos is None:
        self._feed(d)
      else:
        self._feed(d[:pos])  # backlog
        self._live = True
        self._feed(d[pos:])
      self._cv.notify_all()

  def _feed(self, d):
    for ev in self._scanner.feed(d):
      if ev[0] == 'oversize':
        self._error('framing_oversize',
                    f'framing: frame exceeded {proto.MAX_FRAME} bytes, discarded {ev[1]} bytes')
        continue
      enc = ev[1]
      try:
        payload = proto.cobs_decode(enc)
      except proto.CobsError:
        self._error('framing_invalid',
                    f'framing: invalid COBS ({len(enc)} bytes) near {bytes(enc[:16]).hex()}…')
        continue
      self._on_payload(payload)

  def _find_pulse(self, d):
    # The hub sends a pulse after the backlog replay (protocol.md §8). Frames carry both
    # delimiters, so two frames are separated by exactly two 00s: a longer run of 00s holds a
    # pulse, and so does a run of two at the start of the stream.
    for i, b in enumerate(d):
      if b == 0:
        self._zeros += 1
        if self._zeros >= (2 if self._stream_start else 3):
          return i + 1
      else:
        self._zeros = 0
        self._stream_start = False
    return None

  def _record(self, target, level, file, line, text, error=None):
    return LogRecord(time.monotonic() - self._start, level, target, file, line, text, error)

  def _error(self, kind, text, target=None):
    self._push(self._logs, self._record(target, 'ERROR', '<uclog>', '', text, kind), 'logs')

  def _on_payload(self, p):
    m = proto.mux_decode(p)
    if m[0] == 'pulse':
      return
    if m[0] == 'port' and m[1] == proto.PORT_DEVICE_INFO:
      self._on_device_info(m[2])
    if not self._live:
      return
    if m[0] == 'port' and m[1] == proto.PORT_SEMIHOST:
      self._on_semihost(m[2])
      return
    self._push(self._frames, p, 'frames')
    if m[0] == 'error':
      self._error('short_log', f'decode: {m[1]}')
    elif m[0] == 'port':
      self._seq += 1
      self._push(self._ports[m[1]], (self._seq, m[2]), m[1])
    elif m[1] == proto.TYPE_RES:
      self._error('reserved_type', f'decode: reserved frame type 2 [{_hex(p)}]', m[2])
    elif self._pending is not None:
      self._pending.append(m)
    else:
      self._decode(m)

  def _decode(self, m):
    _, _, target, addr, args = m
    ld = self._decoders.get(target)
    if ld is None:
      self._error('no_logdata',
                  f'decode: no logdata for target {target} (addr 0x{addr:08x}) [{_hex(args)}]', target)
      return
    try:
      level, file, line, text = ld.decode(addr, args)
      rec = self._record(target, level, file, line, text)
    except DecodeError as e:
      text = (f'decode: unknown format addr 0x{addr & ~3:08x} in {ld.source} [{_hex(args)}]'
              if e.kind == 'unknown_format' else f'decode: {e}')
      rec = self._record(target, 'ERROR', '<uclog>', '', text, e.kind)
    self._push(self._logs, rec, 'logs')

  def _on_device_info(self, data):
    try:
      info = cbor.loads(data)
    except cbor.CborError as e:
      self._error('bad_cbor', f'port 62: invalid CBOR ({e}) [{_hex(data)}]')
      return
    if not isinstance(info, dict):
      return
    self._info = info
    h = info.get('app_hash')
    if not isinstance(h, bytes) or (h == self._app_hash and self._decoders):
      return
    self._app_hash = h
    if self._pending is None:
      self._pending = []
    threading.Thread(target=self._resolve, args=(h,), daemon=True).start()

  def _resolve(self, h):
    lds, source, notes = self._resolver.resolve(h)
    for n in notes:
      log.warning('%s', n)
    with self._cv:
      self.notes.extend(notes)
      if h != self._app_hash:
        return  # superseded by a newer device info
      if lds:
        self._decoders = {ld.target: ld for ld in lds}
        log.info('logdata: %s', source)
        self.notes.append(f'logdata: {source}')
      pending, self._pending = self._pending or [], None
      for m in pending:
        self._decode(m)
      self._cv.notify_all()

  def _on_semihost(self, data):
    r, console = self._semihost.handle(data)
    if console and console[0] == 'stdin':
      n = console[1]
      if self._stdin is None:
        self._enqueue(proto.wire_port(proto.PORT_SEMIHOST, self._semihost.stdin_reply(b'', n)))
      else:
        threading.Thread(target=self._answer_stdin, args=(n,), daemon=True).start()
      return
    if console:
      last = self._console[-1] if self._console else None
      if last and last[0] == console[0] and len(last[1]) < 4096:
        self._console[-1] = (last[0], last[1] + console[1])  # e.g. unbuffered stderr, a byte per write
      else:
        self._push(self._console, console, 'console')
    if r is not None:
      self._enqueue(proto.wire_port(proto.PORT_SEMIHOST, r))

  def _answer_stdin(self, n):
    try:
      line = self._stdin(n)
    except Exception as e:  # the script's callback failed: give the device EOF
      log.warning('stdin callback: %s', e)
      line = b''
    if isinstance(line, str):
      line = line.encode('utf-8')
    self._enqueue(proto.wire_port(proto.PORT_SEMIHOST, self._semihost.stdin_reply(line or b'', n)))


# ---------------------------------------------------------------------------
# connect()
# ---------------------------------------------------------------------------

def _is_loopback(host):
  try:
    return ipaddress.ip_address(host).is_loopback
  except ValueError:
    return host == 'localhost'


def _hub_matches(h, device):
  return device in (h.device, h.serial_number, h.name)


def connect(host: Optional[str] = None, device: Optional[str] = None, *, logdata=None,
            files_dir=None, stdin=None, cache_dir=None, logdata_domain=None, logdata_url=None,
            logdata_token=None, discover_remote: bool = False, approval_timeout: float = 60.0,
            device_timeout: Optional[float] = 5.0, max_queue: int = 10000,
            skip_backlog: bool = False, hub_args=()) -> Connection:
  """Connect to a hub, starting one if none is running.

  - `host`: a hub's address ("192.168.1.20", "labpc.local:9001"); default $UCLOG_HOST.
  - `device`: choose the hub (or start one) for this device: a URI (`serial:/dev/ttyACM0`) or a
    serial number.
  - `logdata`: a file or a list of files (`.logdata` or ELF). See `Resolver` for the order.
  - `files_dir`: answer the device's file syscalls inside this directory (else ENOSYS).
  - `stdin`: callable(max_bytes) → bytes/str for the device's stdin reads (else EOF).
  - `device_timeout`: for hubs on this machine, how long to wait for the first bytes from the
    device before raising DeviceNotRespondingError (None: don't wait).
  - `approval_timeout`: for remote hubs, how long the hub may hold the connection while its
    operator approves it.
  - `skip_backlog`: drop the frames the hub buffered before this connection.
  """
  if isinstance(logdata, (str, os.PathLike)):
    logdata = [logdata]
  resolver = Resolver(logdata or (), cache_dir, logdata_domain, logdata_url, logdata_token)
  manual = resolver.manual()  # a bad path fails here, before a hub is started
  host = host or os.environ.get('UCLOG_HOST')
  hub = None
  if not host:
    hubs = list_hubs(discover_remote)
    if device:
      hubs = [h for h in hubs if _hub_matches(h, device)]
    if len(hubs) > 1:
      raise MultipleHubsError(
        'several hubs are running; choose one with host= (or UCLOG_HOST):\n' +
        '\n'.join(f'  {h}' for h in hubs), hubs)
    if hubs:
      host = hubs[0].address
    else:
      if device and ':' in device:
        uri = device
      else:
        devs = list_devices()
        if device:
          devs = [d for d in devs if device in (d.serial, d.uri, d.name)]
        if not devs:
          raise NoHubError('no hub is running and no device was found' +
                           (f' matching {device!r}' if device else ''))
        if len(devs) > 1:
          raise MultipleHubsError(
            'several devices found; choose one with device=:\n' +
            '\n'.join(f'  {d}' for d in devs), devs)
        uri = devs[0].uri
      hub = start_hub(uri, hub_args)
      host = f'127.0.0.1:{hub.port}'
  addr, port = parse_host(host)
  try:
    sock = socket.create_connection((addr, port), timeout=10)
  except OSError as e:
    if hub:
      hub.stop()
    raise NoHubError(f'cannot connect to the hub at {addr}:{port}: {e}') from None
  sock.settimeout(None)
  sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
  local = _is_loopback(sock.getpeername()[0])
  c = Connection(sock, resolver=resolver, files_dir=files_dir, stdin=stdin, max_queue=max_queue,
                 skip_backlog=skip_backlog, hub=hub, address=f'{addr}:{port}', manual=manual)
  wait = device_timeout if local else approval_timeout
  if wait is None:
    return c
  with c._cv:
    c._cv.wait_for(lambda: c._first_rx is not None or c._closed, wait)
    got, closed = c._first_rx is not None, c._closed
  if got:
    return c
  c.close()
  if closed:
    raise HubBusyError(f'{addr}:{port}: the hub closed the connection: another client is '
                       'connected (one client at a time), or the hub refused this one')
  if local:
    raise DeviceNotRespondingError(f'{addr}:{port}: no data from the device within {wait} s')
  raise ApprovalTimeoutError(f'{addr}:{port}: no data within {wait} s; the hub may be waiting '
                             'for its operator to approve this connection')
