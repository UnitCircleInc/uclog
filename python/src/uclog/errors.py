# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Exceptions raised by the library."""

__all__ = [
  'UclogError', 'NoHubError', 'MultipleHubsError', 'HubBusyError', 'ApprovalTimeoutError',
  'DeviceNotRespondingError', 'HubStartError', 'ConnectionClosedError',
]


class UclogError(Exception):
  """Base class."""


class NoHubError(UclogError):
  """No hub is running and none could be started (no device, or no `uclog-hub` executable).
  `devices` lists the devices that were found, if any."""

  def __init__(self, msg, devices=()):
    super().__init__(msg)
    self.devices = list(devices)


class MultipleHubsError(UclogError):
  """More than one hub (or device) matches. `candidates` lists them; pass `host=` (or set
  UCLOG_HOST) or `device=` to choose."""

  def __init__(self, msg, candidates):
    super().__init__(msg)
    self.candidates = list(candidates)


class HubBusyError(UclogError):
  """The hub closed the connection at once: another client is connected (a hub serves one
  client at a time), or the hub refused this one."""


class ApprovalTimeoutError(UclogError):
  """A remote hub held the connection open without data while its operator was asked to
  approve it, and nobody approved it in time."""


class DeviceNotRespondingError(UclogError):
  """Connected to the hub, but the device sent nothing."""


class HubStartError(UclogError):
  """A hub started by the library exited or did not report that it was ready."""


class ConnectionClosedError(UclogError):
  """The connection to the hub is closed."""
