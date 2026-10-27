# From device code to screen

A log call in C on the device, and what every host shows for it. The output below was captured
from the [test firmware](https://github.com/UnitCircleInc/uclog/tree/main/firmware) on a
NUCLEO-L452RE-P.

## How a log travels

```c
LOG_INFO("tick %u, uptime %.1f s", count, now / 1000.0);
```

1. **At build time** the format string goes into a section of the ELF that is never loaded into
   flash (`LOGDATA`). The device keeps only its address.
2. **At run time** the call sends that address and the arguments in binary: a few bytes, not the
   text. Nothing is formatted on the device.
3. **After linking**, the build reads the ELF and writes the **logdata**: every format string,
   the kind of each argument, the enums and the symbols. It is embedded in the ELF and copied to
   the hosts' cache, keyed by the **app hash** (SHA-512 of the image), which the device also
   reports when a host connects.
4. **The host** looks up the logdata by that hash and formats the log:

```
0.044  INFO  main.c:64  tick 4, uptime 3.0 s
```

The time is when the host received it; the file and line come from the call site. So a log costs
the device a few bytes on the link, the strings cost no flash, and the logs of an image can only
be decoded with that image's logdata (see [Adding ucLog to a device](index.md)).

## Levels

| Call | Shown as |
|---|---|
| `LOG_DEBUG("debug level")` | `DEBUG  debug level` |
| `LOG_INFO("info level, no arguments")` | `INFO  info level, no arguments` |
| `LOG_WARN("warn level")` | `WARN  warn level` |
| `LOG_ERROR("error level")` | `ERROR  error level` |
| `LOG_FATAL("fatal requested")` | `FATAL  fatal requested`, then the device sends everything it has buffered and resets (or stops at a breakpoint under a debugger) |

On Zephyr, `LOG_DBG()`, `LOG_INF()`, `LOG_WRN()` and `LOG_ERR()` are the same calls
(`CONFIG_LOG_CUSTOM_HEADER`), so existing Zephyr code logs through ucLog unchanged.

## Arguments

`printf` conversions, with the argument's size and signedness taken from its C type at compile
time (`_Generic`): length modifiers are accepted but not needed. Each call, then what the hosts
show:

```c
LOG_INFO("int: %d %d %i, unsigned: %u", 0, -123456, 2147483647, 4000000000u);
// int: 0 -123456 2147483647, unsigned: 4000000000

LOG_INFO("hex: %x %08X, octal: %o, char: %c", 0xbeefu, 0x1234abu, 8u, 'A');
// hex: beef 001234AB, octal: 10, char: A

LOG_INFO("short %hd, unsigned char %hhu, bool %d, long %ld", (short)-2, (unsigned char)250, true, -7L);
// short -2, unsigned char 250, bool 1, long -7

LOG_INFO("int64: %lld %llu %llx", -9000000000000000000LL, 18000000000000000000ULL, 0x0123456789abcdefULL);
// int64: -9000000000000000000 18000000000000000000 123456789abcdef

LOG_INFO("float %f, double %.10f, exp %e, general %g %g", 1.5f, 3.14159265358979, 6.02214076e23, 0.0001, 1e-10);
// float 1.500000, double 3.1415926536, exp 6.022141e+23, general 0.0001 1e-10

LOG_INFO("[%5d] [%-5d] [%+d] [%05.1f] [%#x] [%#o] [%10s] [%-6s]", 42, 42, 42, 3.14159, 255u, 8u, "right", "left");
// [   42] [42   ] [+42] [003.1] [0xff] [010] [     right] [left  ]

LOG_INFO("string: \"%s\", empty \"%s\"", "hello", "");
// string: "hello", empty ""

LOG_INFO("escapes \"%s\"", "tab\there\nnew\\line\x01");
// escapes "tab\there\nnew\\line\x01"

LOG_INFO("utf-8: %s", "café ✓");
// utf-8: café ✓

LOG_INFO("pointer %p, size_t %zu, percent 100%%", (void *)&heartbeat, sizeof(bytes));
// pointer 0x20000c60, size_t 20, percent 100%

LOG_INFO("14 args: %d %d %d %d %d %d %d %d %d %d %d %d %d %d", 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14);
// 14 args: 1 2 3 4 5 6 7 8 9 10 11 12 13 14
```

Strings are copied into the log, up to the frame's size (`LOG_MAX_LOG_SIZE`, 100 bytes by
default); a longer one is cut short.

## Enums and symbols

Two extensions to `printf` use what the build read from the ELF's debug information:

```c
LOG_INFO("enum: {enum:app_state_t}%d {enum:sensor_mode}%d", app_state, MODE_FAST);
// enum: APP_BUSY MODE_FAST

LOG_INFO("unknown value {enum:app_state_t}%d", 7);
// unknown value <app_state_t:7>

LOG_INFO("function {sym}%p, variable {sym}%p", (void *)log_selftest, (void *)&heartbeat);
// function log_selftest+0x0, variable src/main.c:heartbeat+0x0

LOG_INFO("unknown {sym}%p", (void *)0x12345678);
// unknown 0x12345678
```

`{enum:name}` takes an `enum name` or a typedef'd enum; `{sym}` finds the function or variable
an address points into, which helps with callbacks, fault addresses and buffers.

## Memory

```c
LOG_MEM_INFO("bytes", bytes, sizeof(bytes));
```

```
INFO  src/main.c:133  bytes 08009d08: 00 01 02 7f 80 ff 75 63 4c 6f 67 00 de ad be ef 55 aa 0d 0a
```

`LOG_MEM_DEBUG` … `LOG_MEM_FATAL`: the label, the address, and the bytes in hex.

## Beyond logs

The same link carries other ports, which every host shows in its own view:

- **Terminal** (port 61): text both ways. The device registers a handler with `log_notify()` and
  answers with `log_tx()`; the [examples](index.md#the-examples) take `led on` and `led off`
  this way.

    ```c
    log_notify(61, on_terminal, NULL);                  // lines typed in the Terminal view
    log_tx(61, (const uint8_t *)"ok\r\n", 4);           // text shown there
    ```

- **Plots** (port 60, `log_plot.h`): line plots with up to three y axes of nine series each.

    ```c
    static const char *const trig[] = {"sin", "cos"};
    const log_plot_axis_t axes[] = {{.label = "value", .series = trig, .n = 2, .lo = -1.1f, .hi = 1.1f}};
    log_plot_create("waves", "t", axes, 1, 0);
    const float ys[] = {sinf(t), cosf(t)};
    log_plot_point("waves", t, ys, 2);                  // one x, one y per series
    ```

- **Semihost** (port 63): with the library's system calls (`UCLOG_SYSCALLS_SRCS`, `uclog_syscalls`
  or `CONFIG_UC_SYSCALLS`), `printf()` goes to the host's terminal view, `stdin` reads the next
  line typed there, and `fopen()` reads and writes files in a folder the host chose
  (`uclog term --files-dir`, *Serve files from folder…* in the web app, `uclog.semihost.root` in
  VS Code).
- **Commands** (port 0): CBOR requests and replies, for the hosts' command buttons and for
  scripts (the Python library's `command()`).
- **Firmware update** (ports 59 and 3), from any host.

The wire format of each is in the [protocol](../protocol/index.md).
