# ucLog device library

The device side of [ucLog](../README.md): compact binary logs, terminal and command ports, and
semihost files and stdio, over a UART, USB (CDC, HID) or BLE link to the ucLog hosts (`uclog`, the
hub, the web app, VS Code, Python). Wire format: [spec/protocol.md](../spec/protocol.md).

One library for bare-metal firmware and for Zephyr (as a Zephyr module). Version:
`UCLOG_DEVICE_VERSION` in [include/log.h](include/log.h), sent as `uclog_version` in the device
info; tagged `device-vN` ([spec/compatibility.md](../spec/compatibility.md)).

| Path | Contents |
|---|---|
| [include/log.h](include/log.h) | The API: `LOG_DEBUG`…`LOG_FATAL`, `LOG_MEM_*`, `log_tx`/`log_rx`/`log_notify`, device info |
| [include/log_port.h](include/log_port.h) | The port (link) interface a board or RTOS implements |
| [include/semihost.h](include/semihost.h) | The semihost client (port 63): files and stdio on the host |
| [src/log.c](src/log.c) | The library: framing, logs, host presence, RX, device info, saved log |
| [src/semihost.c](src/semihost.c), [src/syscalls.c](src/syscalls.c) | Semihost requests; newlib syscalls on them (bare metal) |
| [ports/zephyr/](ports/zephyr/) | The Zephyr port: `log_port.h` over the drivers below, the log thread, `SYS_INIT`, Zephyr's `LOG_*` as ucLog logs, the shell backend, semihost as a file system |
| [ports/baremetal/log.ld](ports/baremetal/log.ld) | Linker fragment for the format strings (bare metal) |
| [drivers/](drivers/), [dts/](dts/) | Zephyr drivers: `ucuart_z` (any Zephyr UART), `ucuart` and `ucusb` (nRF UARTE and USBD) |
| [cmake/](cmake/), [snippets/](snippets/), [zephyr/module.yml](zephyr/module.yml), [Kconfig](Kconfig) | Zephyr build integration |
| [tools/](tools/README.md) | Build-time tools: logdata from the ELF, the app hash, image signing |
| [app/](app/) | A minimal Zephyr sample |

COBS, CBOR and the ring buffer come from [ucComm](https://github.com/UnitCircleInc/ucComm) (a
Zephyr module, or its `src/` on bare metal).

## Bare metal

Compile `src/log.c` (and `src/semihost.c` + `src/syscalls.c` for semihost with newlib) with
`include/` on the include path, and implement `log_port.h` for your link. Then:

```c
int main(void) {
  log_pre_init();                 // logs are buffered from here on
  board_init();
  log_init("my_board", "app");    // board name and image type go into the device info
  for (;;) {
    log_poll();                   // frames from the host, port handlers
    ...
  }
}
void SysTick_Handler(void) { log_tick(++ms); } // 1 kHz: pulses and host presence
```

Link with `INCLUDE log.ld` inside `SECTIONS` and a `LOGDATA` memory region (`ORIGIN = 0xF0000000`,
the target digit is `(origin >> 20) & 0xF`), and a 64-byte `.apphash` section in flash. After
linking: `tools/hash.py -f` on the binary, write the digest into `.apphash`, then `tools/logdata.py`
on the ELF (see `../firmware/*/Makefile`). The test firmware in [../firmware](../firmware/README.md)
does all of this for three STM32 boards.

## Zephyr

Add this directory as a Zephyr module (west, or `ZEPHYR_EXTRA_MODULES`) together with ucComm, pick
the link in the devicetree, and enable `CONFIG_UC_LOG`:

```dts
/ { chosen { uclog,link = &uclog0; }; };
&usart2 {
	uclog0: uclog { compatible = "staric,ucuart_z"; current-speed = <1000000>; };
};
```

```
CONFIG_UC_LOG=y
CONFIG_LOG=y
CONFIG_LOG_MODE_MINIMAL=y
CONFIG_LOG_CUSTOM_HEADER=y   # LOG_INF() and friends become ucLog logs
```

The port starts the library itself (`log_pre_init()` at `PRE_KERNEL_1`, the link and the log thread
at `APPLICATION`), so the app does not call `log_init()`. The device info's board is `CONFIG_BOARD`;
override `get_image_type()` for the image type. Logs need the `LOGDATA` region: the
`uclog-target0`/`uclog-target1` snippets, or a `zephyr,memory-region` node as in
`../firmware/zephyr/logdata.overlay`. The build adds the app hash, embeds the logdata
(`.logdata_cbor`) and copies it to the hosts' cache.

| Option | |
|---|---|
| `CONFIG_UC_LOG_BUF_SIZE` | the log ring (default 8 KiB) |
| `CONFIG_UC_SYSCALLS` | semihost: stdout/stdin to the host's console, the host's files mounted at `CONFIG_UC_SYSCALLS_MOUNT` (`/host`) |
| `CONFIG_UC_SHELL` | the Zephyr shell on port 0 (snippet `uclog-shell`) |
| `CONFIG_UC_LOG_SAVE` | keep the log ring across a reset (`log_saved_log()`) |
| `CONFIG_UC_SIGNED_IMAGE` | the app hash from the signed image's header (`sbl_app_hash()`) |
| `CONFIG_UC_LOG_SERVER`, `CONFIG_UC_LOG_SERVER_PORTS` | no effect since `device-v4` (the host link is always on) |

Notes:
- Tested with Zephyr 3.7 (NCS 2.9) and 4.3. `ucuart_z` is tested on hardware (NUCLEO-L452RE-P);
  `ucuart` and `ucusb` (nRF) are build-tested only.
- On a USART without a FIFO (STM32L4 and others) at 1 Mbaud, give `ucuart_z`'s UART DMA with a
  **circular** RX channel (`STM32_DMA_MODE_CYCLIC`): interrupt-driven RX, and DMA that switches
  between buffers, lose bytes. See `../firmware/zephyr/boards/`.
- The host has no seek, stat, unlink or rename. The semihost file system keeps file positions
  itself (so `fstat()` and `ftell()` work), and stats a path by opening it.
- Zephyr 4.x with `CONFIG_POSIX_API`: its `fdopen()` returns a zvfs object, not a picolibc `FILE`,
  so picolibc's `fopen()` does not work on any file system there. Use `open()`/`read()`/`write()`
  or `fs_open()`.
