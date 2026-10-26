# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""ucLog for scripts: connect to a device through the ucLog hub, send and receive on any port,
and decode logs.

    import uclog

    with uclog.connect(logdata="build/zephyr/zephyr.elf") as dev:
        print(dev.device_info)
        dev.send(61, b"version s\\r")
        print(dev.recv(61, timeout=1.0))
        for rec in dev.logs(timeout=5.0):
            print(rec)

All device I/O goes through the native hub (`uclog-hub`); a hub is started when none is running
and stopped when the script exits.
"""

from . import aio, cbor, proto
from .connection import Connection, connect
from .errors import (ApprovalTimeoutError, ConnectionClosedError, DeviceNotRespondingError,
                     HubBusyError, HubStartError, MultipleHubsError, NoHubError, UclogError)
from .hub import DeviceInfo, HubInfo, list_devices, list_hubs, start_hub, stop_hub
from .logdata import DecodeError, LogData, LogDataError, LogRecord
from .resolver import Resolver

__version__ = '4a1'

__all__ = [
  'connect', 'Connection', 'list_hubs', 'list_devices', 'start_hub', 'stop_hub', 'HubInfo',
  'DeviceInfo', 'LogData', 'LogRecord', 'Resolver', 'DecodeError', 'LogDataError', 'UclogError',
  'NoHubError', 'MultipleHubsError', 'HubBusyError', 'ApprovalTimeoutError',
  'DeviceNotRespondingError', 'HubStartError', 'ConnectionClosedError', 'aio', 'cbor', 'proto',
]
