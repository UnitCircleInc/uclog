# ucLog

ucLog is compact binary logging for embedded devices, and the tools that read it. A log call on
the device sends only the address of its format string and the raw arguments, a few bytes, and
the host turns them back into text using the strings it reads from the firmware's ELF file. The
same link also carries a terminal, commands, plots, files on the host (semihost) and firmware
updates.

<div class="grid cards" markdown>

- **[Open the web app](https://uclog.unitcircle.ca/app/)**: connect to a device over Web Serial,
  WebHID or Web Bluetooth, or to a `uclog hub`, from Chrome or Edge. It works offline once loaded.
- **[Getting started](getting-started.md)**: install the hosts and connect to a device.
- **[The device library](https://github.com/UnitCircleInc/uclog/tree/main/device)**: bare metal
  or Zephyr, over a UART, USB or BLE.
- **[The wire protocol](protocol/index.md)**: framing, ports, logdata and the test vectors that
  every implementation passes.

</div>

## The hosts

| Host | Use it for |
|---|---|
| Web app | Nothing to install; Chrome or Edge on any desktop |
| VS Code extension | Logs next to the code: click a log to open its source line |
| `uclog gui`, `uclog tui` | A native window or a terminal UI; `uclog term` for a plain terminal |
| `uclog hub` | Share one device with a client anywhere on the network, or with the web app |
| Python (`pip install uclog`) | Tests and scripts that read logs and send commands |

ucLog is open source (Apache-2.0) and developed by [Unit Circle](https://www.unitcircle.ca).
