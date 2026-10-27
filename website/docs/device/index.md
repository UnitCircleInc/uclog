# Adding ucLog to a device

The device library is C, for bare-metal firmware and for Zephyr. This page describes the library
as released with these docs (`device-v4`). Its source is `device/` in the
[ucLog repository](https://github.com/UnitCircleInc/uclog/tree/main/device), mirrored on its own as
[uclog-device](https://github.com/UnitCircleInc/uclog-device) for projects that only need it.

A project needs four things:

1. **The library**: `src/log.c` and ucComm (COBS, CBOR, a ring buffer), included.
2. **A link**: a port for your UART, USB or BLE stack (bare metal), or a devicetree node (Zephyr).
3. **Two linker additions**: a `LOGDATA` region for the format strings, which is never loaded,
   and a 64-byte `.apphash` field in flash.
4. **A post-link step**: it writes the app hash into the image and makes the logdata from the
   ELF.

The [examples](#the-examples) do all four in a few files, bare metal and Zephyr, on a
NUCLEO-L452RE-P. How the logs are decoded: [From device code to screen](logging.md).

## Bare metal

### Start-up and the main loop

```c
int main(void) {
  log_pre_init();                  // logs are kept from here, before the link is up
  board_init();                    // clocks, the 1 kHz tick
  log_init("my_board", "app");     // board name and image type, sent to the host
  for (;;) {
    log_poll();                    // frames from the host; port handlers run here
    ...
  }
}

void SysTick_Handler(void) {
  log_tick(++ms);                  // 1 kHz: pulses and host presence
}
```

Logs are kept in an 8 KiB ring (`LOG_TX_BUF_SIZE`) until a host connects, and sent from then on;
the link is quiet while no host is there.

### The port

The board implements `include/log_port.h`, the link under the library:

| Function | |
|---|---|
| `log_port_init(cb_t *tx)` | start the link; TX takes its data from `tx`, the library's ring |
| `log_port_tx_schedule(prefix, n)` | start sending: `prefix` (if any), then what is in the ring. Usually a DMA transfer of the ring's contiguous part, restarted from its completion interrupt |
| `log_port_tx_pause(bool)`, `log_port_tx_idle()` | stop starting transfers (no host); no transfer in progress |
| `log_port_panic()` | switch to polled TX with interrupts off (`LOG_FATAL`) |
| `log_port_rx_avail()`, `_peek()`, `_skip(n)` | the received bytes, as a ring the library reads |
| `log_port_rx_mark()` | a value that changes whenever a byte arrives (a DMA counter) |
| `log_port_rx_check()` | clear UART errors; nonzero if there were any |
| `log_port_lock()`, `_unlock()`, `log_port_ms()` | a critical section, and a millisecond clock |
| `log_port_fatal()` | stop at a breakpoint under a debugger, else reset |

The example's `port.c` implements it for a UART with DMA in about 250 lines of CMSIS register
code, the board's clock and tick included. At high baud rates, receive with DMA into a circular buffer: a USART without a FIFO loses
bytes at 1 Mbaud when it is read from an interrupt.

### The linker script

```ld
MEMORY
{
  FLASH (rx)  : ORIGIN = 0x08000000, LENGTH = 512K
  RAM (rwx)   : ORIGIN = 0x20000000, LENGTH = 160K
  LOGDATA     : ORIGIN = 0xF0000000, LENGTH = 1M     /* not loaded: the format strings */
}
SECTIONS
{
  ...
  .apphash : { . = ALIGN(4); KEEP(*(.apphash)) } > FLASH    /* written after linking */
  ...
  INCLUDE log.ld                                            /* places .logdata in LOGDATA */
}
```

`LOGDATA`'s address also names the image: its target digit is `(ORIGIN >> 20) & 0xF`, so a
bootloader at `0xF0000000` (target 0) and an app at `0xF0100000` (target 1) can both log, and the
host tells their logs apart.

### Building

=== "Makefile"

    ```make
    UCLOG_DIR := path/to/device
    include $(UCLOG_DIR)/uclog.mk

    SRCS    += $(UCLOG_SRCS) $(UCLOG_SYSCALLS_SRCS)   # the library; newlib system calls (semihost)
    CFLAGS  += $(UCLOG_CFLAGS) -mfp16-format=ieee     # include paths; ucComm needs the fp16 format
    LDFLAGS += $(UCLOG_LDFLAGS)                       # so the linker script finds log.ld

    $(BUILD)/app.elf: $(OBJS)
    	$(CC) $(LDFLAGS) $(OBJS) -o $@.tmp
    	$(call uclog_post_link,$@.tmp,$@,$(BUILD))
    ```

=== "CMake"

    ```cmake
    add_subdirectory(path/to/device uclog)
    # or:
    # include(FetchContent)
    # FetchContent_Declare(uclog GIT_REPOSITORY https://github.com/UnitCircleInc/uclog-device GIT_TAG v4)
    # FetchContent_MakeAvailable(uclog)

    target_link_libraries(app PRIVATE uclog uclog_syscalls)
    uclog_add_logdata(app)
    ```

    CMake compiles absolute paths, which become the file names the logs show: map them with
    `-ffile-prefix-map=${CMAKE_CURRENT_SOURCE_DIR}/=` (the library maps its own to `uclog/…`).

=== "By hand"

    Compile `src/log.c`, `src/log_plot.c`, `src/semihost.c`, optionally `src/syscalls.c`, and
    `uccomm/{cb,cobs,cbor,crc32c}.c`, with `include/` and `uccomm/` on the include path (Arm GCC:
    `-mfp16-format=ieee`). After linking, run what `uclog.mk`'s `uclog_post_link` runs:
    `tools/hash.py`, `objcopy --update-section .apphash=…`, `tools/logdata.py`, and
    `objcopy --add-section .logdata_cbor=…`.

Compile with `-g`: the logdata takes argument types, enums and symbols from the debug
information.

`uclog_syscalls` (`UCLOG_SYSCALLS_SRCS`) implements newlib's system calls over the link, so
`printf()`, `stdin` and files reach the host (semihost). Without it, link newlib's stubs
(`--specs=nosys.specs`) as usual.

## Zephyr

The library is a Zephyr module. Add it to your `west.yml`:

```yaml
manifest:
  projects:
    - name: uclog-device
      url: https://github.com/UnitCircleInc/uclog-device
      revision: v4
      path: modules/uclog
```

or build with `-DZEPHYR_EXTRA_MODULES=path/to/device`. Then choose the link in the devicetree:

```dts
/ {
	chosen {
		uclog,link = &uclog0;
	};
};

&usart2 {
	current-speed = <1000000>;
	uclog0: uclog {
		compatible = "staric,ucuart_z";   /* any Zephyr UART */
		current-speed = <1000000>;
	};
};
```

and add the `LOGDATA` region with a snippet, `west build -S uclog-target0` (or `uclog-target1`
for target 1). The snippet also sets ucLog's Kconfig:

```
CONFIG_UC_LOG=y
CONFIG_LOG=y
CONFIG_LOG_MODE_MINIMAL=y
CONFIG_LOG_CUSTOM_HEADER=y   # Zephyr's LOG_INF() and friends become ucLog logs
CONFIG_CONSOLE=n
CONFIG_UART_CONSOLE=n
```

The library starts itself (no init call). The build adds the app hash, embeds the logdata and
copies it to the hosts' cache.

| Option | |
|---|---|
| `CONFIG_UC_LOG_BUF_SIZE` | the log ring (default 8 KiB) |
| `CONFIG_UC_SYSCALLS` | semihost: stdout and stdin to the host's terminal, the host's files at `CONFIG_UC_SYSCALLS_MOUNT` (`/host`) |
| `CONFIG_UC_SHELL` | the Zephyr shell on the command port (snippet `uclog-shell`) |
| `CONFIG_UC_LOG_SAVE` | keep the log ring across a reset (`log_saved_log()`) |
| `CONFIG_UC_SIGNED_IMAGE` | the app hash from a signed image's header |

Notes:

- The device CI builds with Zephyr 3.7 LTS and 4.4.
- Logs name files relative to the app (`src/main.c`), the West workspace (`zephyr/...`,
  `modules/...`) or the library (`uclog/...`): the module sets `-fmacro-prefix-map` for these, so
  no absolute path ends up in the firmware.
- On an STM32L4 (a USART without a FIFO) at 1 Mbaud, give `ucuart_z` DMA with a **circular** RX
  channel, as the example's overlay does: interrupt-driven RX, and DMA from one buffer to the
  next, lose bytes.
- The drivers: `ucuart_z` (any Zephyr UART; tested on hardware), and `ucuart` and `ucusb` (nRF
  UARTE and USB device; build-tested).

## Links

The library sends and receives bytes; the link is the port's business. The hosts connect over:

| Link | On the device | Reference |
|---|---|---|
| UART (an ST-LINK or J-Link virtual COM port, a USB serial adapter) | a UART, ideally with DMA | the examples; every test board |
| USB CDC-ACM | a USB device stack's CDC class | the NUCLEO-C071RB test firmware |
| USB HID | a vendor-defined interface, 63 bytes per report | the NUCLEO-C071RB test firmware |
| Bluetooth LE | a GATT service with one characteristic: notifications out, writes in | the STM32WBA55G-DK1 test firmware |

The hosts' side of each link, and the frame format, are in the [protocol](../protocol/index.md#4-links).
A board can serve several links: the test firmware for the C071 answers on UART, CDC and HID at
once, its port switching to whichever the host uses.

## What the build produces

| File | |
|---|---|
| the ELF | with the logdata embedded (`.logdata_cbor`): give the ELF to a host and it decodes the logs |
| `<app_hash>.logdata` | the logdata alone, also copied to the hosts' cache (`~/.cache/uclog/`, or `$UCLOG_CACHE_DIR`) |
| the binary | what you flash; its app hash is what the device reports |

A host finds the logdata by the device's app hash: a file you loaded, then its cache, then a
logdata service if one is set up. Rebuilding changes the hash, so each build's logs are decoded
with that build's strings.

## The examples

Both log every second and answer `led on`, `led off` and `led blink` on the terminal. They are
built by the device CI and were checked on the board with the hosts. Source:
[`device/examples/`](https://github.com/UnitCircleInc/uclog/tree/main/device/examples).

### Bare metal

```sh
cd device/examples/baremetal
make STM32CUBE_L4=path/to/STM32CubeL4     # or: cmake -B build -DCMAKE_TOOLCHAIN_FILE=cortex-m4f.cmake
make flash
uclog tui                                  # or any host
```

=== "main.c"

    ```c
    --8<-- "device/examples/baremetal/main.c"
    ```

=== "port.c"

    ```c
    --8<-- "device/examples/baremetal/port.c"
    ```

=== "Makefile"

    ```make
    --8<-- "device/examples/baremetal/Makefile"
    ```

=== "CMakeLists.txt"

    ```cmake
    --8<-- "device/examples/baremetal/CMakeLists.txt"
    ```

=== "stm32l452re.ld"

    ```ld
    --8<-- "device/examples/baremetal/stm32l452re.ld"
    ```

### Zephyr

```sh
west build -b nucleo_l452re/stm32l452xx/p device/examples/zephyr -- -DZEPHYR_EXTRA_MODULES=$PWD/device
west flash
```

=== "src/main.c"

    ```c
    --8<-- "device/examples/zephyr/src/main.c"
    ```

=== "prj.conf"

    ```
    --8<-- "device/examples/zephyr/prj.conf"
    ```

=== "boards/nucleo_l452re_stm32l452xx_p.overlay"

    ```dts
    --8<-- "device/examples/zephyr/boards/nucleo_l452re_stm32l452xx_p.overlay"
    ```

=== "CMakeLists.txt"

    ```cmake
    --8<-- "device/examples/zephyr/CMakeLists.txt"
    ```
