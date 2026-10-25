# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""asyncio API: the same calls as the sync one, awaited.

    async with await uclog.aio.connect(logdata="build/zephyr/zephyr.elf") as dev:
        await dev.send(61, b"version s\\r")
        print(await dev.recv(61, timeout=1.0))
        async for rec in dev.logs(timeout=5.0):
            print(rec)

The connection's I/O runs on its own threads (see `uclog.Connection`); waits run in the
default executor so the event loop is never blocked.
"""

import asyncio
import functools
from typing import AsyncIterator, Optional

from . import connection
from .logdata import LogRecord

__all__ = ['connect', 'Connection']


async def _run(fn, *args, **kw):
  loop = asyncio.get_running_loop()
  return await loop.run_in_executor(None, functools.partial(fn, *args, **kw))


class Connection:
  """Async wrapper of `uclog.Connection`. `sync` is the wrapped connection."""

  def __init__(self, sync: connection.Connection):
    self.sync = sync

  async def __aenter__(self):
    return self

  async def __aexit__(self, *exc):
    await self.close()
    return False

  @property
  def alive(self) -> bool:
    return self.sync.alive

  @property
  def closed(self) -> bool:
    return self.sync.closed

  @property
  def dropped(self):
    return self.sync.dropped

  @property
  def notes(self):
    return self.sync.notes

  async def device_info(self, timeout: Optional[float] = None) -> Optional[dict]:
    """The port-62 map. Without `timeout`, waits as long as the sync `device_info` property."""
    return await _run(self.sync.wait_device_info,
                      self.sync._info_timeout() if timeout is None else timeout)

  async def send(self, port: int, data) -> None:
    self.sync.send(port, data)

  async def send_frame(self, payload: bytes) -> None:
    self.sync.send_frame(payload)

  async def recv(self, port: int, timeout: Optional[float] = None) -> Optional[bytes]:
    return await _run(self.sync.recv, port, timeout)

  async def console(self, timeout: Optional[float] = None):
    return await _run(self.sync.console, timeout)

  async def command(self, obj: dict, timeout: float = 2.0):
    return await _run(self.sync.command, obj, timeout)

  async def logs(self, timeout: Optional[float] = None, duration: Optional[float] = None) -> AsyncIterator[LogRecord]:
    it = self.sync.logs(timeout, duration)
    while True:
      r = await _run(next, it, None)
      if r is None:
        return
      yield r

  async def frames(self, timeout: Optional[float] = None, duration: Optional[float] = None) -> AsyncIterator[bytes]:
    it = self.sync.frames(timeout, duration)
    while True:
      r = await _run(next, it, None)
      if r is None:
        return
      yield r

  async def close(self) -> None:
    await _run(self.sync.close)


async def connect(*args, **kw) -> Connection:
  """Same arguments as `uclog.connect()`."""
  return Connection(await _run(connection.connect, *args, **kw))
