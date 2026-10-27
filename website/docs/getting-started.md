# Getting started

1. **Pick a host.** The quickest is the [web app](web-app.md): open it in Chrome or Edge. For the
   others: the VS Code extension from the Marketplace, `pip install uclog` for Python, or the
   `uclog` and `uclog-hub` downloads for Linux, macOS and Windows.
2. **Try the simulated device.** In the web app, choose *Connect…* and *Simulated device*: it logs,
   answers on the terminal (type `help`) and draws plots, with no hardware.
3. **Add the device library to your firmware.** See the
   [device library](https://github.com/UnitCircleInc/uclog/tree/main/device) for bare metal
   (Makefile or CMake) and Zephyr (a Zephyr module).
4. **Connect.** Plug the board in and choose it in *Connect…*. The host finds the logdata (the
   format strings) from the device's app hash: in the ELF you load, the logdata cache that the
   firmware build fills, or a logdata service.
