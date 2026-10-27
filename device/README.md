# ucLog device library

> In **UnitCircleInc/uclog-device**: this is a read-only mirror of the `device/` directory of
> [UnitCircleInc/uclog](https://github.com/UnitCircleInc/uclog), updated on every push there, where issues and pull requests
> go. Its `vN` tags are the device library's `device-vN` releases (`v1`–`v3`: the repository
> before it moved).

The device side of [ucLog](https://github.com/UnitCircleInc/uclog/blob/main/README.md): compact binary logs, terminal and command ports, and
semihost files and stdio, over a UART, USB (CDC, HID) or BLE link to the ucLog hosts (`uclog`, the
hub, the web app, VS Code, Python). Wire format: [spec/protocol.md](https://github.com/UnitCircleInc/uclog/blob/main/spec/protocol.md).

One library for bare-metal firmware and for Zephyr (as a Zephyr module). Version:
`UCLOG_DEVICE_VERSION` in [include/log.h](include/log.h), sent as `uclog_version` in the device
info; tagged `device-vN` ([spec/compatibility.md](https://github.com/UnitCircleInc/uclog/blob/main/spec/compatibility.md)).

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
| [CMakeLists.txt](CMakeLists.txt), [uclog.mk](uclog.mk) | Builds: Zephyr module, or a CMake library; a Makefile fragment |
| [cmake/](cmake/), [snippets/](snippets/), [zephyr/module.yml](zephyr/module.yml), [Kconfig](Kconfig), [west.yml](west.yml) | Zephyr build integration; the post-link steps for CMake |
| [uccomm/](uccomm/README.md) | ucComm v3 (COBS, CBOR, the ring buffer), pinned |
| [test/host/](test/host/) | The library on a host, checked against the protocol's test vectors |
| [tools/](tools/README.md) | Build-time tools: logdata from the ELF, the app hash, image signing |
| [app/](app/) | A minimal Zephyr sample |

COBS, CBOR and the ring buffer come from [ucComm](https://github.com/UnitCircleInc/ucComm), tag v3:
the copy in `uccomm/`, or on Zephyr the ucComm module when the workspace has one.

## Bare metal

Implement `log_port.h` for your link, and build the library one of three ways:

- **Makefile**: `UCLOG_DIR := path/to/device`, `include $(UCLOG_DIR)/uclog.mk`, then add
  `$(UCLOG_SRCS)` (and `$(UCLOG_SYSCALLS_SRCS)` for semihost with newlib) to the sources,
  `$(UCLOG_CFLAGS)` and `$(UCLOG_LDFLAGS)` to the flags, and `$(call uclog_post_link,...)` after
  the link (comments in [uclog.mk](uclog.mk)).
- **CMake**: `add_subdirectory(path/to/device)`, or FetchContent from this mirror (or from the ucLog
  repository with `SOURCE_SUBDIR device`); then `target_link_libraries(app PRIVATE uclog
  uclog_syscalls)` and `uclog_add_logdata(app)` (comments in [CMakeLists.txt](CMakeLists.txt)).
- **By hand**: `src/log.c`, `src/semihost.c`, `src/syscalls.c` and `uccomm/{cb,cobs,cbor,crc32c}.c`,
  with `include/` and `uccomm/` on the include path (Arm GCC: `-mfp16-format=ieee` for ucComm).

Then:

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
linking (done by `uclog_post_link` and `uclog_add_logdata`): `tools/hash.py -f` on the binary,
write the digest into `.apphash`, then `tools/logdata.py` on the ELF, embedded as `.logdata_cbor`.
The [test firmware](https://github.com/UnitCircleInc/uclog/blob/main/firmware/README.md) does all of this for three STM32 boards.

## Zephyr

Add the library as a Zephyr module, pick the link in the devicetree, and enable `CONFIG_UC_LOG`. In
your `west.yml` (the mirror is small; the ucLog repository works too, with the module at its root):

```yaml
    - name: uclog-device
      url: https://github.com/UnitCircleInc/uclog-device
      revision: v4
      path: modules/uclog
```

or `-DZEPHYR_EXTRA_MODULES=path/to/device`. ucComm comes from `uccomm/` unless the workspace has the
ucComm module.

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
[firmware/zephyr/logdata.overlay](https://github.com/UnitCircleInc/uclog/blob/main/firmware/zephyr/logdata.overlay). The build adds the app
hash, embeds the logdata (`.logdata_cbor`), and copies it to the hosts' cache and the build
directory as `<app_hash>.logdata`.

| Option | |
|---|---|
| `CONFIG_UC_LOG_BUF_SIZE` | the log ring (default 8 KiB) |
| `CONFIG_UC_SYSCALLS` | semihost: stdout/stdin to the host's console, the host's files mounted at `CONFIG_UC_SYSCALLS_MOUNT` (`/host`) |
| `CONFIG_UC_SHELL` | the Zephyr shell on port 0 (snippet `uclog-shell`) |
| `CONFIG_UC_LOG_SAVE` | keep the log ring across a reset (`log_saved_log()`) |
| `CONFIG_UC_SIGNED_IMAGE` | the app hash from the signed image's header (`sbl_app_hash()`) |
| `CONFIG_UC_LOG_SERVER`, `CONFIG_UC_LOG_SERVER_PORTS` | no effect since `device-v4` (the host link is always on) |

Notes:
- Built with Zephyr 3.7 LTS (3.7.2) and 4.4 (4.4.2), as the device CI does, and with NCS 2.9.
  `ucuart_z` is tested on hardware (NUCLEO-L452RE-P, Zephyr 4.3); `ucuart` and `ucusb` (nRF) are
  build-tested only.
- On a USART without a FIFO (STM32L4 and others) at 1 Mbaud, give `ucuart_z`'s UART DMA with a
  **circular** RX channel (`STM32_DMA_MODE_CYCLIC`): interrupt-driven RX, and DMA that switches
  between buffers, lose bytes. See [firmware/zephyr/boards/](https://github.com/UnitCircleInc/uclog/blob/main/firmware/zephyr/boards).
- The host has no seek, stat, unlink or rename. The semihost file system keeps file positions
  itself (so `fstat()` and `ftell()` work), and stats a path by opening it.
- Zephyr 4.x with `CONFIG_POSIX_API`: its `fdopen()` returns a zvfs object, not a picolibc `FILE`,
  so picolibc's `fopen()` does not work on any file system there. Use `open()`/`read()`/`write()`
  or `fs_open()`.
