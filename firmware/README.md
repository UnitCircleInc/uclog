# Firmware

A bare-metal ucLog device library and a test firmware for exercising the hosts on real hardware.

| Directory | Contents |
|---|---|
| [`uclog/`](uclog/) | Device library: `log.h`, `log.c`, `log.ld`, plus `tools/logdata.py` and `tools/apphash.py` |
| [`uccomm/`](uccomm/) | COBS, CBOR, CRC-32C and the ring buffer (`cb`), copied unchanged from [UnitCircleInc/ucComm](https://github.com/UnitCircleInc/ucComm) `src/` (commit 8bc8e3b) |
| [`nucleo-l452re-p/`](nucleo-l452re-p/) | Test firmware for the NUCLEO-L452RE-P (STM32L452RE, Cortex-M4F, hard-float) |

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
    runs over 1450 bytes or invalid COBS (ucComm `cobs_dec`).
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

**Semihost**: `src/sh/syscalls.c` is the Unit Circle newlib syscalls file, with Zephyr removed.
Changes, marked "uclog-clientv2":
- Calls that need a reply fail at once with `EIO` when no host is connected. Otherwise the request
  would reach a later host as a stale request; for example, `setvbuf()` calls `_fstat` at boot.
  `_write` (no reply) still buffers.
- `_read` requests the capped length; the original requested the caller's length.
- `_read` keeps `errno` on failure; the original always cleared it.
- `_fstat` reads `st_uid` from bytes 24–27; the original read byte 27 twice.

**STM32CubeL4**: the CMSIS and LL headers, the startup file and `system_stm32l4xx.c` come from an
STM32CubeL4 package:
- the default path is `~/STM32Cube/Repository/STM32Cube_FW_L4_V1.18.2`;
- override it with `STM32CUBE_L4=…`, for example to use a checkout of
  [STM32CubeL4](https://github.com/STMicroelectronics/STM32CubeL4).

The build also needs `-mfp16-format=ieee`, because ucComm CBOR uses `__fp16`.

```sh
cd firmware/nucleo-l452re-p
make            # build/app.elf (+ .logdata_cbor), app.bin, app.logdata, <app_hash>.logdata
make flash      # STM32_Programmer_CLI over SWD
make run        # flash, then: uclog term -l build/app.elf --files-dir build/files
```

**Toolchain**:
- `arm-none-eabi-gcc` on PATH, otherwise STM32CubeCLT (`CUBECLT=/opt/ST/STM32CubeCLT_1.20.0`).
- `python3`.

**Build steps**:
1. Link the ELF.
2. Hash the image: `app_hash` = SHA-512 of the `.bin` with the `.apphash` field zeroed.
3. Write the hash into the field.
4. Generate the logdata.
5. Embed the logdata as `.logdata_cbor`.

`build/<app_hash>.logdata` lets `--cache-dir build` resolve the logdata from the device info
without `-l`.

**Files**: logs come from `src/main.c`, `src/drivers/sensor.c`, `src/sh/*.c`, `board/*.c` and
`../uclog/log.c`, so file names can be checked in the decoded output.

**Commands**: these work on port 61 (`uclog term`) and port 0 (`uclog term --port 0`). Port 5
echoes every frame back and logs its size and CRC-32C. Port 5 is unassigned in the spec and is
used only by this test firmware.

| Command | What it exercises |
|---|---|
| `test` | One log of every kind: int32/uint32/int64/uint64, float/double, flags and widths, strings (escapes, UTF-8, truncation), pointer, `{sym}` (function, variable, unknown), `{enum}` (enum, typedef enum, unknown value), 14 arguments, `LOG_MEM`, all levels. It also runs at boot |
| `sh` | Semihost through `syscalls.c` (printf/stderr with no-reply WRITE, `fopen`/`fgets`/`fstat`, `mkdir`, `gettimeofday`), plus a 1438-byte and a 4000-byte file round trip checked with CRC-32C. It also sends raw requests: the time ops, a WRITE with a reply, and the expected errors (`EACCES` sandbox escape, `EBADF` for close and write, `ENOSYS` for LSEEK and an unknown op, `EINVAL` for a malformed request, the 48-byte FSTAT error) |
| `input` | Semihost READ on stdin |
| `burst [n]` | Throughput. It waits for buffer room, so nothing is dropped. The reply gives bytes, bytes per log and B/s, timed until the last byte has gone out |
| `big [n]` | One port-61 frame of `n` bytes (default 1443, which is 1452 on the wire) |
| `info`, `stats` | Resends the device info; shows the link counters |
| `led on/off`, `fatal` | LED control; `LOG_FATAL` (flush, then reset) |

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

**Not tested on hardware**: a device receiving a frame over the limit. `uclog` refuses to send
one, so a raw serial tool would be needed.
