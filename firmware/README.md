# Firmware

A bare-metal ucLog device library and a test firmware for exercising the hosts on real hardware.

| Directory | Contents |
|---|---|
| [`uclog/`](uclog/) | Device library: `log.h`, `log.c`, `log.ld`, plus `tools/logdata.py` and `tools/apphash.py` |
| [`uccomm/`](uccomm/) | COBS, CBOR, CRC-32C and the ring buffer (`cb`), copied unchanged from [UnitCircleInc/ucComm](https://github.com/UnitCircleInc/ucComm) `src/` (commit 8bc8e3b) |
| [`app/`](app/) | The test app every board builds: commands, the selftests, semihost (`syscalls.c`) |
| [`nucleo-l452re-p/`](nucleo-l452re-p/) | Test firmware for the NUCLEO-L452RE-P (STM32L452RE, Cortex-M4F, hard-float): UART |
| [`nucleo-c071rb/`](nucleo-c071rb/) | Test firmware for the NUCLEO-C071RB (STM32C071RB, Cortex-M0+): UART, USB CDC-ACM and USB HID at once |
| [`stm32wba55g-dk1/`](stm32wba55g-dk1/) | Test firmware for the STM32WBA55G-DK1 (STM32WBA55CG, Cortex-M33): UART and BLE at once |

## Device library (`uclog/`)

This is the core of `include/log.h`, `lib/log.c`, `lib/logserver.c`, `lib/log.ld` and
`lib/log.cmake` from [UnitCircleInc/uclog](https://github.com/UnitCircleInc/uclog), with Zephyr
removed. App signatures and FW update are not included.

- **Logging API** (unchanged): `LOG_DEBUG/INFO/WARN/ERROR/FATAL`, `LOG_MEM_*`, `{enum:name}%d` and
  `{sym}%p`.
  - Each call sends the address of its format string (placed in the non-loaded `.logdata`
    section), followed by the packed arguments.
  - The argument kinds are computed at compile time with `_Generic`.
- **Link API** (as in the Zephyr version):
  - `log_pre_init()`, then `log_init(board, image_type)`.
  - `log_tx(port, …)` sends; `log_rx(port, buf, n)` waits for a reply.
  - `log_notify(port, cb, ctx)` registers a handler for frames on a port.
  - `log_poll()` goes in the main loop; `log_tick(ms)` goes in a 1 kHz tick.
- **Port interface**: shaped like the Zephyr `ucuart` API, so a board supplies:
  - a TX side that drains the log ring buffer: `log_port_init(tx_cb)`,
    `log_port_tx_schedule(prefix, n)`, `log_port_tx_pause()`, `log_port_panic()`;
  - an RX ring read with `avail`/`peek`/`skip`, plus `rx_mark` (changes on every byte) and
    `rx_check` (restart after UART errors);
  - `lock`/`unlock`, `ms` and `fatal`.

### Behaviour (spec/protocol.md)

- **Frame size**: `LOG_MAX_PACKET_SIZE` is **1443** (it was 1500), so every frame is at most
  **1452 bytes on the wire**. That is one unfragmented UDP datagram on a 1500-byte MTU with IPv6
  (§2.4).
- **TX**:
  - Frames are COBS-encoded on the stack (`log_tx` uses a static buffer under the lock), then
    copied into an 8 KiB `cb_t` ring buffer, which the port drains.
  - Every frame starts with `00`. A pulse is sent after 0.5 s of TX idle.
- **Host presence**:
  - The port is paused 2 s after the device last received a byte (the host pulses every 0.5 s).
  - Logs are kept until a host connects. When the ring is full, new frames are dropped and
    counted, and a "frames dropped" warning is logged later.
  - When bytes arrive again, the device skips the rest of any frame that was cut off. It then
    sends the port-62 device info (`app_hash`, `board`, `image_type`, packed with ucComm
    `cbor_pack`), followed by the backlog.
  - An empty port-62 frame from the host asks for the device info again (F7). It is sent
    straight away, so a host that reconnects within 2 s still gets it.
- **RX**:
  - `log_poll()` splits the RX ring into frames: it drops bytes before the first `00` and rejects
    runs over 1450 bytes, invalid COBS (ucComm `cobs_dec`) and frames that decode to more than
    1444 bytes (a 1450-byte run can decode to 1449). Rejected frames are counted in `rx_errors`.
  - Each frame is dispatched to the handler for its port, or to a waiting `log_rx()`.
- **Fatal**: `LOG_FATAL` switches the port to polled TX with interrupts off and flushes the whole
  ring. It then executes `bkpt` if a debugger is attached, or resets.

### Differences from the Zephyr version

- `log_rx` returns 0 when the host goes away (instead of waiting forever), or when it is called
  from a handler.
- There is one handler slot for each of the 64 ports; the Zephyr version had 8.
- Polled `log_poll()` replaces the log thread.
- `long double` arguments are sent as 8-byte doubles. On ARM, `long double` is a `double`; the
  Zephyr version copied 16 bytes.
- When the host is absent, logs are buffered rather than sent.

### Logdata (`tools/logdata.py`)

`tools/logdata.py` produces the `.logdata` CBOR (spec §6.1) from an ELF file. It needs only the
Python standard library: it reads ELF sections itself and reads DWARF through
`arm-none-eabi-objdump -Wi`, as the original did.

- **Same output as the original**:
  - `%p` → `0x%08x`
  - `{sym}`/`{enum:}` → `%s`
  - `ll` → `l`
  - the same kinds
- **Additional length modifiers**: it also accepts `hh h j t L z` (spec §6.2). `%hd` and `%jd`
  work, which the original's regex skipped.

## Test app (`app/`)

Both boards build the same app; a board supplies `board.h` (`board_init`, the LED, `board_ms`,
`board_uid`, and optionally `board_link`) and the ucLog port interface. Boards with little RAM
shrink the buffers with `-D`: `LOG_TX_BUF_SIZE` (the log ring), `SH_BUF_SIZE` (the largest semihost
request or reply) and `SH_BIG_FILE` (the selftest's multi-call file).

**Semihost**: `sh/syscalls.c` is the Unit Circle newlib syscalls file, with Zephyr removed.
Changes, marked "uclog-clientv2":
- Calls that need a reply fail at once with `EIO` when no host is connected. Otherwise the request
  would reach a later host as a stale request; for example, `setvbuf()` calls `_fstat` at boot.
  `_write` (no reply) still buffers.
- `_read` requests the capped length; the original requested the caller's length.
- `_read` keeps `errno` on failure; the original always cleared it.
- `_fstat` reads `st_uid` from bytes 24–27; the original read byte 27 twice.
- Reply lengths are clamped to the request buffer (`log_rx` returns the payload length, which can
  be longer than what it copied), and `_read` copies at most the requested length.
- `_isatty` returns 0 with `errno` set on failure; the original returned -1, which newlib takes as
  "a tty".

**Files**: logs come from `src/main.c`, `src/drivers/sensor.c`, `src/sh/*.c` (the app, built with
`-ffile-prefix-map=../app/=src/`), `board/*.c` and `../uclog/log.c`, so file names can be checked in
the decoded output.

**Commands**: these work on port 61 (`uclog term`) and port 0 (`uclog term --port 0`). Port 5
echoes every frame back and logs its size and CRC-32C. Port 5 is unassigned in the spec and is
used only by this test firmware.

| Command | What it exercises |
|---|---|
| `test` | One log of every kind: int32/uint32/int64/uint64, float/double, flags and widths, strings (escapes, UTF-8, truncation), pointer, `{sym}` (function, variable, unknown), `{enum}` (enum, typedef enum, unknown value), 14 arguments, `LOG_MEM`, all levels. It also runs at boot |
| `sh` | Semihost through `syscalls.c` (printf/stderr with no-reply WRITE, `fopen`/`fgets`/`fstat`, `mkdir`, `gettimeofday`), plus a file round trip of one maximum WRITE (1438 bytes; 507 on the C071) and one of several (4000 bytes; 1200 on the C071), checked with CRC-32C. It also sends raw requests: the time ops, a WRITE with a reply, and the expected errors (`EACCES` sandbox escape, `EBADF` for close and write, `ENOSYS` for LSEEK and an unknown op, `EINVAL` for a malformed request, the 48-byte FSTAT error) |
| `input` | Semihost READ on stdin |
| `burst [n]` | Throughput. It waits for buffer room, so nothing is dropped. The reply gives bytes, bytes per log and B/s, timed until the last byte has gone out |
| `big [n]` | One port-61 frame of `n` bytes (default 1443, which is 1452 on the wire) |
| `info`, `stats` | Resends the device info; shows the link counters |
| `link` | The link in use: `uart`, `cdc` or `hid` |
| `led on/off`, `fatal` | LED control; `LOG_FATAL` (flush, then reset) |

## NUCLEO-L452RE-P test firmware

**Board setup**:
- 80 MHz from HSI16 + PLL, with the FPU enabled.
- USART2 on PA2/PA3 (the ST-LINK virtual COM port) at **1 Mbaud**.
- LED LD2 on PB13: it blinks slowly with no host and fast with one.

**UART driver**: `board/uart.c`, `dma.c` and `gpio.c` are the Unit Circle STM32L4 drivers (LL
based).
- **RX**: circular DMA (DMA1 channel 6) into a 4 KiB ring. Interrupt latency can no longer lose
  received bytes.
- **TX**: linear DMA (channel 7) straight from the log ring buffer.
- **Changes**, each marked "uclog-clientv2" or described at the top of the file:
  - the scheduler notify is now a callback;
  - a TX prefix (for the device info) and a TX pause;
  - a fix: a panic-mode flush with nothing to send waited forever for TC.
  - `{flags:uart_error_t}` became `{enum:uart_error_t}`, because `{flags:}` is not in the spec.
- **`board.c`**: `uart_init()` resets its TX buffer, so the board passes it an empty one and
  attaches the log ring afterwards. Otherwise the logs from before `uart_init` were lost.

**STM32CubeL4**: the CMSIS and LL headers, the startup file and `system_stm32l4xx.c` come from an
STM32CubeL4 package:
- the default path is `~/STM32Cube/Repository/STM32Cube_FW_L4_V1.18.2`;
- override it with `STM32CUBE_L4=…`, for example to use a checkout of
  [STM32CubeL4](https://github.com/STMicroelectronics/STM32CubeL4).

The build also needs `-mfp16-format=ieee`, because ucComm CBOR uses `__fp16`.

```sh
cd firmware/nucleo-l452re-p
make            # build/app.elf (+ .logdata_cbor), app.bin, app.logdata, <app_hash>.logdata (+ cache)
make flash      # STM32_Programmer_CLI over SWD
make run        # flash, then: uclog term -l build/app.elf --files-dir build/files
```

`make flash` programs the board's own ST-LINK: `STLINK_SN=…`, or the probe whose board name is
`NUCLEO-L452RE-P` (several boards may be connected).

The toolchain and build steps are the same for every board.

**Toolchain**:
- `arm-none-eabi-gcc` on PATH, otherwise STM32CubeCLT (`CUBECLT=/opt/ST/STM32CubeCLT_1.20.0`).
- `python3`.

**Build steps**:
1. Link the ELF.
2. Hash the image: `app_hash` = SHA-512 of the `.bin` with the `.apphash` field zeroed.
3. Write the hash into the field.
4. Generate the logdata.
5. Embed the logdata as `.logdata_cbor`.

The build also copies `<app_hash>.logdata` into the hosts' logdata cache, `UCLOG_CACHE` (default
`$UCLOG_CACHE_DIR`, else `$XDG_CACHE_HOME/uclog`, else `~/.cache/uclog`: the same rule as `uclog`
and the Python library, and the directory the Zephyr module's `cachelogdata.py` writes to). So
`uclog tui`, `uclog gui` and the other hosts find the logdata from the device info without `-l`.
`build/<app_hash>.logdata` stays, for `--cache-dir build`.

### Results (2026-10-23, DMA build, `uclog` with the 1452-byte limit, serial via the ST-LINK VCP)

Everything decoded correctly:
- every `test` line;
- `sh` passed: all 7 expected errors, the file round trips matched by CRC-32C, and the host's copy
  of the file matched too;
- `input` round-tripped a typed line.

| Check | Result |
|---|---|
| `burst 5000` | 95,000 bytes (19.0 per log) in 950 ms = 100,000 B/s, the full 1 Mbaud line rate (8N1); 0 dropped. All 5,000 decoded on the host |
| 7 queued commands, repeated runs | Every reply arrived; `rx_errors` 0, `rx_uart_errors` 0, `tx_dropped` 0 |
| Largest frame, device → host | `big`: 1443 bytes received, 1452 on the wire, CRC-32C matching |
| Largest frame, host → device | Port 5: 1443 bytes received with matching CRC-32C, then echoed back |
| Over the limit | The host refuses 1444 bytes (`send_oversize`); `big 1444` is refused on the device |
| Logdata resolution | Found from the device info `app_hash` in the cache directory, without `-l` |
| Backlog | Logs written while no host was connected arrived on the next connection, after the device info |
| `fatal` | The FATAL line was flushed, the board reset, and the boot logs (including `log-pre-init`) followed with no framing errors |

**Over the limit, host → device** (2026-10-24, raw serial with pyserial): a 1450-byte run that
decodes to 1449 bytes is rejected and counted in `rx_errors`, on all three boards. Before the fix it
reached the port-5 handler and reset the board (`FATAL tx message too long 1448`).

## NUCLEO-C071RB test firmware

**Board setup**:
- 48 MHz from HSIUSB48 (also the USB clock, trimmed to the host's start-of-frame by the CRS).
- USART2 on PA2/PA3 (the ST-LINK virtual COM port, `0483:3752`) at **1 Mbaud**, DMA through DMAMUX
  (RX: channel 1, circular, 2 KiB ring; TX: channel 2 from the log ring). `board/uart.c` has the
  L452 driver's behaviour (TX prefix and pause, panic flush, error restart) without its low-power,
  RS-485 and flow-control parts.
- The user USB connector: a composite device `2FE3:0100`, serial number = the chip's unique ID
  (`board/usb.c`, a small stack for the STM32 USB DRD FS peripheral):

  | Interface | Function | Endpoints |
  |---|---|---|
  | 0 | Placeholder vendor HID (usage page FF02): the ucLog HID must be interface 1 (protocol.md §4.3) and CDC-ACM needs two adjacent interfaces | 0x81 interrupt IN, 8 bytes, never sends |
  | 1 | **ucLog HID**: usage page FF00, input report A1 and output report A2, 63 bytes each | 0x82 IN / 0x02 OUT, interrupt, 64 bytes, every frame |
  | 2-3 | **CDC-ACM** (with an interface association): the host sees a serial port | 0x83 notifications (unused), 0x84 IN / 0x04 OUT bulk, 64 bytes |

  - TX: packets are filled from the log ring across its wrap point, so the HID's zero padding
    only ever follows whole frames. A bulk transfer that ends with a full packet gets a
    zero-length packet.
  - RX: each function has a 512-byte ring; an OUT packet that does not fit stays NAKed until
    there is room (USB flow control).
  - HID `GET_REPORT` (the fallback when interrupt reads fail) returns an empty report; `SET_REPORT`
    output reports are accepted like interrupt OUT ones.
- LED LD1 on PA5.
- 24 KiB of RAM: `LOG_TX_BUF_SIZE=4096`, `SH_BUF_SIZE=512`, `SH_BIG_FILE=1200`, a 2 KiB stack. The
  image is about 55 KiB of flash and 20 KiB of static RAM.

**Three links, one in use**: the firmware listens on all three. The link a host talks on is used;
another link takes over when it receives bytes and the one in use has been silent for 2 s (the
device library's host timeout). So the same image is tested over the ST-LINK UART, CDC and HID
without rebuilding, one host at a time. `link` says which is in use, and the change is logged
(`link: hid`). Bytes on the links not in use are discarded.

**STM32CubeC0**: the CMSIS core and device headers, the LL headers and `system_stm32c0xx.c` come
from an STM32CubeC0 package:
- the default path is `~/STM32Cube/Repository/STM32Cube_FW_C0_V1.4.1` (install it with STM32CubeMX);
- override it with `STM32CUBE_C0=…`, for example to use a checkout of
  [STM32CubeC0](https://github.com/STMicroelectronics/STM32CubeC0) with its submodules
  (`Drivers/CMSIS/Device/ST/STM32C0xx` and `Drivers/STM32C0xx_HAL_Driver` are enough);
- Zephyr's `modules/hal/stm32/stm32cube/stm32c0xx` also works (detected by its `soc/` folder), with
  `CMSIS_CORE=…` set to a CMSIS 5 `Include/` folder, since it has no `core_cm0plus.h`.

There is no ST startup file: `board/startup.c` has the vector table and reset handler.
`SystemCoreClockUpdate()` does not recognise HSIUSB48 as the system clock, so the board sets
`SystemCoreClock` itself.

```sh
cd firmware/nucleo-c071rb
make            # or make STM32CUBE_C0=…
make flash      # this board's ST-LINK (board name NUCLEO-C071RB, or STLINK_SN=…)
```

### Results (2026-10-23, macOS, NUCLEO-C071RB with both USB connectors attached)

`uclog-hub list` shows the three links: `serial:/dev/cu.usbmodem…` (ST-LINK `0483:3752`),
`serial:/dev/cu.usbmodem…` (CDC, `2fe3:0100`) and `hid:<serial>`.

| Check | UART | CDC | HID |
|---|---|---|---|
| `test` (Python library, through `uclog-hub`) | 23 records, 0 error records | same | same |
| `burst 2000` | 2,000 decoded, 0 errors, 100,000 B/s | 2,000, 0 errors, ~350,000 B/s | 2,000, 0 errors, ~63,000 B/s (one report per ms) |
| 1443-byte frame, device → host (`big`) | ✓ | ✓ | ✓ |
| 1443-byte frame, host → device (port 5 echo) | ✓ | ✓ | ✓ |
| `sh` directly (`uclog term`, no hub): files, CRC-32C round trips, the 7 expected errors | ✓ | ✓ | ✓ |
| VS Code (`web/apps/vscode/e2e/hw.spec.ts`): workspace logdata, `test`, `burst 5000`, `sh` into the workspace | 5,052 logs, 0 errors | 5,150, 0 errors | 5,041, 0 errors |
| Device info on connect, link handover after 2 s | ✓ | ✓ | ✓ |

Found on the way:
- The serial filter lacked `0483:3752` (ST-LINK/V2-1 without mass storage, on this board); it is
  now in protocol.md §4.1 and every client.
- The first HID build split frames: a report taken from the ring's contiguous part ended at the
  wrap point and was zero padded. Packets are now filled across the wrap.
- The web client chose any HID interface with VID 2FE3; it now takes the one with input report A1
  (a composite device's other HID interfaces are listed separately by the browser).
- (2026-10-24) 20 back-to-back 100-byte port-5 frames over HID lost the first 3 echoes and gave a
  framing error. The native HID link wrote every report of a batch before reading again; a write
  waits while the device's RX is full, and hidapi on macOS drops the oldest input reports beyond
  30 queued. It now reads between reports: 20/20, 20 × 1443 and 100 × 200 bytes all intact.

### Why HID and CDC are slower than the bus (2026-10-25)

Measured on macOS. The C071 is a full-speed device behind two high-speed Genesys hubs
(`05e3:0610`). The C071 test firmware was temporarily instrumented: a raw-frame burst, cycle
counts from SysTick, and a delay in the TX-complete interrupt.

**HID: ~63 kB/s is the full-speed limit for one interrupt endpoint.**
- A full-speed interrupt endpoint carries at most one transaction of at most 64 bytes per 1 ms
  frame (USB 2.0 §5.7.3–5.7.4). The endpoint already asks for every frame (`bInterval` 1).
- Report ID `A1` takes one byte of each report: 63 bytes × 1000 = 63,000 B/s, which is the rate
  measured. Host to device (output report `A2`) has the same limit.
- Neither the firmware nor the host can raise it. Only more interrupt endpoints (protocol.md §4.3
  has one HID interface) or a high-speed device would. HID is there for hosts without a serial
  driver; CDC is the fast USB link.

**CDC: ~350 kB/s is the CPU, not USB.**
- Full-speed bulk allows up to 19 packets of 64 bytes per frame (~1.2 MB/s), and ~1 MB/s is
  normal for a full-speed device. Here the Cortex-M0+ at 48 MHz runs out of time first:

  | Measured | Cycles | Time |
  |---|---|---|
  | TX-complete interrupt, one full 64-byte packet: copy from the log ring to a stack buffer, then byte-assembled 32-bit writes into the packet memory | ~1,900 | 40 µs |
  | `log_tx()` of a 63-byte frame, interrupts off: copy, COBS, ring write (~40 cycles per byte) | ~3,400 | 71 µs |
  | `log_tx()` of a 1,000-byte frame, interrupts off | ~40,600 | 845 µs |

- A stream of 63-byte frames needs ~111 µs of CPU per 64-byte packet, a ceiling of ~580 kB/s; it
  measured 519 kB/s.
- Large frames are slower (1,000-byte frames: 410 kB/s). Interrupts stay off for the whole encode
  and the endpoint has one buffer, so only the packet already armed goes out in those 845 µs.
- `burst` (19-byte logs, formatted and encoded one at a time) measured 336 kB/s.
- Confirmed by two experiments:
  - A delay added in the TX-complete interrupt lowered the rate at once (10 µs: 519 → 453 kB/s;
    50 µs: 286 kB/s), so there is no idle time to absorb it.
  - Copying words instead of bytes into the packet memory cut the interrupt to ~1,150 cycles.
    Throughput rose 10–16%: 63-byte frames 604 kB/s, 1,000-byte frames 452 kB/s, `burst`
    362 kB/s.

  Neither change was kept.
- Ways to get closer to 1 MB/s (plan 07 task 36):
  - copy words straight from the ring into the packet memory, without the stack buffer;
  - a double-buffered bulk IN endpoint, so a second packet is armed while the CPU is busy. It
    needs its own CHEP register, since double buffering is one direction only;
  - encode with only the ring locked, not all interrupts;
  - a faster COBS encoder.
- The UART links don't have this problem: there DMA drains the ring, not the CPU, and 1 Mbaud
  (100,000 B/s) is far below these costs.

**Not tested**: Web Serial and WebHID with this board (the browser's chooser cannot be automated:
Chrome's DevTools `DeviceAccess` domain only covers Bluetooth, and seeded permissions are
ignored), Windows and Linux hosts, and USB suspend/resume (not handled: the device keeps its
state).

## STM32WBA55G-DK1 test firmware

**Board setup**:
- 32 MHz from the HSE (the radio's crystal) as SYSCLK, SMPS, voltage range 1; the LSE clocks the
  radio's sleep timer.
- USART1 on PB12/PA8 (the STLINK-V3 virtual COM port, `0483:374e`) at **1 Mbaud** (`board/uart.c`):
  - RX: circular DMA (GPDMA1 channel 0, one linked-list item that reloads itself) into a 4 KiB
    ring. The first version was interrupt driven with the 8-byte FIFO (80 µs at 1 Mbaud), and the
    ucLog lock masks USART1 while `log_tx()` encodes a frame (~400 µs for the largest): 20
    back-to-back 1443-byte port-5 frames gave 1 intact echo and 10 overruns. Now 20 of 20.
  - TX: interrupt driven with the FIFO, from the log ring.
- **BLE** (`board/ble.c`, protocol.md §4.2): one service and one characteristic, both
  `8683b8f3-0000-1000-8000-00805f9b34fb`, with write, write without response and notify.
  - Advertises as **"ucLog test STM32WBA55G-DK1"**: the 128-bit service UUID in the advertisement,
    the name in the scan response (they do not both fit in 31 bytes); a static random address made from
    the chip's unique ID; 30-50 ms interval; advertising again after a disconnection.
  - Device → host: notifications of ATT MTU − 3 bytes, but at most one link layer packet's worth
    once the data length is extended: data length − 4 (L2CAP) − 3 (ATT) = **244** bytes of 251 (a
    248-byte notification takes a second packet for its last 4 bytes). Filled from the log ring
    across its wrap point, queued until the stack reports it is full (then the rest waits for
    its "TX pool available" event). The stack's packet pool has 512 extra 32-byte blocks (the
    example's 21 hold about two full notifications), room for ~60 waiting notifications; the
    controller has 8 ACL buffers of 251 bytes.
  - Log data is fragmented into full notifications: while the log keeps coming only full ones are
    sent (each notification costs a link layer packet, whatever its size). A short one goes once
    the ring has not grown for a millisecond (SysTick), or for the device info.
  - Host → device: writes of any size, including 512-byte long writes (Web Bluetooth), into a
    2 KiB RX ring. A value longer than one stack event arrives in several, in order.
  - On connection the device asks for data length extension (251 bytes, 2120 µs) and the 2M PHY;
    after 1 s (earlier requests are refused while the first procedures run) for a 15 ms
    connection interval, latency 0, 2 s supervision timeout (L2CAP connection parameter update). The outcome is logged: `ble: data length tx 251 bytes / 2120 us, rx …`,
    `ble: PHY tx 2M, rx 2M`, `ble: connection interval 15.00 ms`.
- ST's BLE stack runs the way its `BLE_p2pServer_Simplest` example uses it (no sequencer, timer
  server or low power), but in the lowest-priority interrupt (TSC's vector, pended by software)
  instead of the main loop. So the link keeps moving while the test app busy-waits (`reply()`,
  `big`). Stack events are queued and handled after `BleStack_Process()`, which must not be
  re-entered.
- Interrupt priorities: the radio 0 (5 while only its sleep timer runs), USART1 2, SysTick 3, the
  BLE stack and the link layer's background work 15. The ucLog lock (`board_lock()`) masks
  priorities 1-15 through BASEPRI, so it never delays the radio. (`disable_irq`/`enable_irq` are
  taken: the link layer library defines its own.)
- LED LD3 on PB15.
- The image is about 150 KiB of flash and 80 KiB of static RAM (the BLE stack's buffers included).

**Two links, one in use**: as on the C071, the link a host talks on is used, and the other takes
over when it receives bytes and the one in use has been silent for 2 s. `link` says which is in use.

**STM32CubeWBA**: the CMSIS headers, HAL/LL, the startup file, the BLE stack and link layer
libraries (`stm32wba_ble_stack_full.a`, `LinkLayer_BLE_Peripheral_Only_V3_0.a`) and the chip-level
platform files (`STM32_WPAN/Target/*.c` and their configuration headers) of the
`NUCLEO-WBA55CG/Applications/BLE/BLE_p2pServer_Simplest` example (the same chip) all come from the
package: the default is `~/STM32Cube/Repository/STM32Cube_FW_WBA_V1.10.0`, override it with
`STM32CUBE_WBA=…`. Nothing from it is copied here; `board/skel_ble.h` stands in for the example's
application header.

```sh
cd firmware/stm32wba55g-dk1
make            # or make STM32CUBE_WBA=…
make flash      # this board's ST-LINK (board name STM32WBA55G-DK1, or STLINK_SN=…)
```

### Results (2026-10-24, macOS)

`uclog list --ble-scan 5` shows `ble:<id>  ucLog test STM32WBA55G-DK1`. Through `uclog-hub` the
device is `ble:ucLog test STM32WBA55G-DK1` (by name).

| Check | UART | BLE |
|---|---|---|
| `test` (Python library, through `uclog-hub`) | 23 records, 0 error records | 24 records, 0 error records |
| `burst 2000` | 2,000 decoded, 0 errors, 100,000 B/s | 2,000 decoded, 0 errors (the 8 KiB ring absorbs part: ~70,000 B/s) |
| Sustained (`burst 10000`) | — | ~72,000 B/s (see below) |
| 1443-byte frame, device → host (`big`) | ✓ | ✓ |
| Host → device (port 5 echo), 10 to 1443 bytes | ✓ | ✓ |
| `sh` through the hub and directly (`uclog term -d ble:…`, no hub) | ✓ | ✓ |
| VS Code (`hw.spec.ts`; BLE: *Scan for Bluetooth devices*, then the name) | 5,056 logs, 0 errors | 5,079 logs, 0 errors |
| Web Bluetooth in Chrome, no hub (`web/e2e/ble.spec.ts`): device info, `test`, `burst 5000`, a 1411-byte command in 512-byte long writes | — | 5,054 logs, 0 errors |
| Device info on connect, link handover after 2 s | ✓ | ✓ |

**BLE throughput with a Mac**: macOS connects at 30 ms, negotiates 251-byte packets and the 2M PHY
when asked, and accepts the 15 ms interval request (asking for 7.5-15 ms also gives 15 ms). Each
connection event then carries about 4.4 notifications of 244 bytes (~1,080 bytes), with the
device's queues full (the stack reports "pool full" throughout a burst):

| Device side | Rate |
|---|---|
| First version: one notification per log (~19 bytes), pool of ~2, 30 ms | ~19,000 B/s |
| Full notifications, large pool, DLE + 2M, 30 ms interval | ~26,000 B/s |
| The same at 15 ms | ~52,000 B/s |
| The same with the 1M PHY | ~27,000 B/s |
| 15 ms, notifications of 244 instead of 248 bytes (one link layer packet each) | ~72,000 B/s |

With 248-byte notifications every one took two link layer packets (251 + 4 bytes), so the "3.3
notifications per event" were really 6-7 packets. The rate halves with the 1M PHY and doubles with
half the interval: macOS limits each connection event by time, not the device. More needs a
central that gives longer connection events (or a shorter interval).

**BLE throughput with Windows 11** (2026-10-25, Intel Wireless Bluetooth, USB `8087:0026`):
Windows negotiates what macOS does (15 ms interval, 2M PHY, 251-byte data length, ATT MTU 251),
so the device sends the same 244-byte notifications. The rate is set by the adapter's driver,
which decides how many packets a connection event carries:

| Intel driver | Notifications per event | `burst 10000` |
|---|---|---|
| 22.190.0.2 (2022) | ~1.8 | 28,581 B/s (`uclog term`), 27,818 B/s (Chrome) |
| 24.70.0.4 (2026) | ~5.0 | 79,714 and 80,936 B/s (`uclog term`), 82,002 B/s (Chrome) |

So on Windows, slow BLE (~28 kB/s with the link at 15 ms and 2M) means an old Bluetooth driver:
install the current one from the adapter's maker (Intel's Driver & Support Assistant), not the one
Windows Update offers.

Found on the way:
- Without `HAL_MspInit()` enabling the PWR clock, the supply and voltage-scaling settings are
  ignored and the radio stays silent (the stack starts and reports advertising, but nothing is on
  the air).
- A full-size stack event (258 bytes) overflowed a one-byte length in the event queue; lengths are
  two bytes now.
- macOS hides BLE addresses (CoreBluetooth reports `00:00:00:00:00:00`): the native client now
  shows no serial for such devices, and `ble:00:00:00:00:00:00` no longer matches every device.
- The stack refuses a connection parameter update request right after connecting (0x0C), so it
  is sent after 1 s.
- The first version sent a notification for every log as it arrived (~19 bytes each), and the
  stack's pool held about two full notifications.

**Not tested**: Linux and Windows hosts, bonding/encryption (none: the characteristic is open),
low power (the firmware never sleeps beyond WFI).
