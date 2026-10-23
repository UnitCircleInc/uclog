# Firmware

A bare-metal ucLog device library and a test firmware for exercising the hosts on real hardware.

| Directory | Contents |
|---|---|
| [`uclog/`](uclog/) | Device library: `log.h`, `log.c`, `log.ld`, plus `tools/logdata.py` and `tools/apphash.py` |
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
- **Link**:
  - `log_tx(port, …)`.
  - `log_notify(port, cb, ctx)` registers a handler for frames on a port.
  - `log_rx(port, buf, n, timeout_ms)` waits for a reply.
  - `log_poll()` goes in the main loop; `log_tick(ms)` goes in a 1 kHz tick.
- **Board interface**: the board implements a few functions:
  - `log_port_lock/unlock`
  - `log_port_tx_start` (enable the TX-empty interrupt)
  - `log_port_tx_poll` (used when panicking)
  - `log_port_ms`
  - `log_port_fatal`

  Its UART interrupt calls `log_uart_tx_next()` and `log_uart_rx(byte, err)`.

### Behaviour (spec/protocol.md)

- **TX**:
  - Frames are COBS-encoded straight into an 8 KiB ring buffer, which the UART TX interrupt
    drains.
  - Interrupts are disabled only while a writer reserves its space and while it publishes,
    **never while it encodes**. At 1 Mbaud a byte arrives every 10 µs, and the STM32L4 USART has
    no RX FIFO. Encoding a 100-byte frame with interrupts off caused RX overruns in testing.
  - Writers can nest (a log from an interrupt). The ring is published when the outermost writer
    finishes.
  - Every frame starts with `00`. A pulse is sent after 0.5 s of TX idle.
- **Host presence**:
  - The device stops draining the ring 2 s after it last received a byte (the host pulses every
    0.5 s). It stops at a frame boundary.
  - Logs are kept until a host connects. When the ring is full, new frames are dropped and
    counted, and a "frames dropped" warning is logged later.
  - The first received byte resumes TX. The port-62 device info (`app_hash`, `board`,
    `image_type`) goes out first, followed by the backlog. Boot logs are therefore never lost.
- **RX**:
  - Bytes go into a 4 KiB ring buffer. `log_poll()` splits them into frames: it drops bytes
    before the first `00` and rejects oversize or invalid COBS.
  - Each frame is dispatched to the handler for its port.
- **Fatal**: `LOG_FATAL` switches to polled TX with interrupts off and flushes the whole ring. It
  then executes `bkpt` if a debugger is attached, or resets.

### Differences from the Zephyr version

- `log_rx` takes a timeout, and returns `LOG_RX_ERROR` when the host goes away or when it is
  called from a handler.
- There is one handler slot for each of the 64 ports; the Zephyr version had 8.
- Polled `log_poll()` replaces the log thread.
- `long double` arguments are sent as 8-byte doubles. On ARM, `long double` is a `double`; the
  Zephyr version copied 16 bytes.
- `log_tx` returns `false` when the ring buffer is full.
- When the host is absent, logs are buffered rather than written to the UART.

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

**Files**: logs come from `src/main.c`, `src/drivers/sensor.c`, `src/sh/semihost.c`,
`board/board.c` and `../uclog/log.c`, so file names can be checked in the decoded output.

**Commands**: these work on port 61 (`uclog term`) and port 0 (`uclog term --port 0`):

| Command | What it exercises |
|---|---|
| `test` | One log of every kind: int32/uint32/int64/uint64, float/double, flags and widths, strings (escapes, UTF-8, truncation), pointer, `{sym}` (function, variable, unknown), `{enum}` (enum, typedef enum, unknown value), 14 arguments, `LOG_MEM`, all levels. It also runs at boot |
| `sh` | Semihost through newlib: `printf`/`stderr`, `fopen`/`fprintf`/`fgets`/`fstat`, `mkdir`, and the time ops. WRITE normally sets bit 31 (no reply), as the Zephyr `syscalls.c` does; one WRITE asks for a reply. It also checks the expected errors: `EACCES` (sandbox escape), `EBADF` (close and write), `ENOSYS` (LSEEK and an unknown op), `EINVAL` (malformed request), and the 48-byte FSTAT error |
| `input` | Semihost READ on stdin |
| `burst [n]` | Throughput. It waits for buffer room, so nothing is dropped |
| `info`, `stats` | Resends the device info; shows the link counters |
| `led on/off`, `fatal` | LED control; `LOG_FATAL` (flush, then reset) |

### Results (2026-10-23, `uclog` from Phase 1, serial via the ST-LINK VCP)

Everything decoded correctly:
- every `test` line;
- `sh` passed: all 7 expected errors, and a file written and read back;
- `input` round-tripped a typed line.

| Check | Result |
|---|---|
| `burst 5000` | 869 ms with 0 dropped, about 5,750 logs/s (the 1 Mbaud line rate). All 5,000 decoded on the host |
| 7 queued commands (`test`, `sh`, `burst 3000`, 4×`stats`), 4 runs | Every reply arrived; `rx_errors` 0, `rx_overrun` 0, `tx_dropped` 0 |
| Logdata resolution | Found from the device info `app_hash` in the cache directory, without `-l` |
| Backlog | Logs written while no host was connected arrived on the next connection, after the device info |
| `fatal` | The FATAL line was flushed, the board reset, and the boot logs followed with no framing errors |
