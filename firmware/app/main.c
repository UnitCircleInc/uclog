// ucLog test firmware, shared by the boards (NUCLEO-L452RE-P, NUCLEO-C071RB).
//
// - At boot, and on `test`: one log of every argument kind, level and format feature.
// - Every second: a heartbeat log with the simulated sensor reading.
// - Port 61 (terminal) and port 0 (text commands): `help` lists the commands.
// - `sh`: semihost selftest (printf, files and time through the host, port 63).
// - Port 5 (test only): echoes each frame back, for maximum-size frames from the host.

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "board.h"
#include "drivers/sensor.h"
#include "log.h"
#include "crc32c.h"
#include "sh/semihost_test.h"

#define TERM_PORT 61
#define CMD_PORT 0
#define ECHO_PORT 5 // test firmware only (an unassigned port)

typedef enum {
  APP_BOOT,
  APP_RUNNING,
  APP_BUSY,
} app_state_t;

static app_state_t app_state = APP_BOOT;
static uint32_t heartbeat;
static bool blink = true;

// Command lines from the port handlers, run by the main loop (handlers must not block).
#define CMD_QUEUE 8
static char line[96];
static size_t line_n;
static struct {
  char text[96];
  int port;
} cmds[CMD_QUEUE];
static uint32_t cmd_head, cmd_tail;

// One port frame, for reply() and big(); static: not on the stack. Neither runs inside the other
// (handlers reply only from log_poll(), which big() does not call).
static uint8_t frame_buf[LOG_MAX_PACKET_SIZE + 1];

// The link the host is using (boards with several links say which).
__attribute__((weak)) const char *board_link(void) {
  return "uart";
}

static void reply(int port, const char *fmt, ...) __attribute__((format(printf, 2, 3)));
static void reply(int port, const char *fmt, ...) {
  char *b = (char *)frame_buf;
  const size_t cap = sizeof(frame_buf);
  va_list ap;
  va_start(ap, fmt);
  int n = vsnprintf(b, cap, fmt, ap);
  va_end(ap);
  if (n < 0) return;
  if ((size_t)n >= cap) n = (int)cap - 1;
  // Replies are not optional like logs: wait (briefly) for the TX interrupt to make room. No
  // log_poll() here: reply() also runs inside port handlers.
  uint32_t t0 = board_ms();
  while (log_tx_avail() < (size_t)n && board_ms() - t0 < 200) { // avail is net of the overhead
  }
  log_tx((uint8_t)port, (const uint8_t *)b, (size_t)n);
}

static void on_term(const uint8_t *rx, size_t n, void *ctx) {
  int port = (int)(intptr_t)ctx;
  // A single byte is a keystroke from a character-mode terminal: echo it. Whole lines (line
  // mode, port 0) are shown by the host already.
  bool echo = port == TERM_PORT && n == 1;
  for (size_t i = 0; i < n; i++) {
    char c = (char)rx[i];
    if (c == '\r' || c == '\n') {
      if (echo) reply(port, "\r\n");
      if (cmd_head - cmd_tail == CMD_QUEUE) {
        reply(port, "busy\r\n");
      } else {
        memcpy(cmds[cmd_head % CMD_QUEUE].text, line, line_n);
        cmds[cmd_head % CMD_QUEUE].text[line_n] = 0;
        cmds[cmd_head % CMD_QUEUE].port = port;
        cmd_head++;
      }
      line_n = 0;
    } else if (c == 0x7F || c == '\b') {
      if (line_n > 0) {
        line_n--;
        if (echo) reply(port, "\b \b");
      }
    } else if (line_n < sizeof(line) - 1) {
      line[line_n++] = c;
      if (echo) log_tx((uint8_t)port, (const uint8_t *)&c, 1);
    }
  }
}

// One log of each argument kind and format feature (decoded output in the comments).
static void log_selftest(void) {
  static const uint8_t bytes[20] = {0x00, 0x01, 0x02, 0x7F, 0x80, 0xFF, 'u', 'c', 'L', 'o',
                                    'g', 0x00, 0xDE, 0xAD, 0xBE, 0xEF, 0x55, 0xAA, 0x0D, 0x0A};
  LOG_DEBUG("debug level");
  LOG_INFO("info level, no arguments");
  LOG_WARN("warn level");
  LOG_ERROR("error level (expected, part of the selftest)");
  LOG_INFO("int: %d %d %i, unsigned: %u, hex: %x %08X, octal: %o, char: %c", 0, -123456, 2147483647,
           4000000000u, 0xbeefu, 0x1234abu, 8u, 'A');
  LOG_INFO("short %hd, unsigned char %hhu, bool %d, long %ld", (short)-2, (unsigned char)250, true,
           -7L);
  LOG_INFO("int64: %lld %llu %llx", -9000000000000000000LL, 18000000000000000000ULL,
           0x0123456789abcdefULL);
  LOG_INFO("float %f, double %.10f, exp %e, general %g %g", 1.5f, 3.14159265358979, 6.02214076e23,
           0.0001, 1e-10);
  LOG_INFO("width/flags: [%5d] [%-5d] [%+d] [%05.1f] [%#x] [%#o] [%10s] [%-6s]|", 42, 42, 42, 3.14159,
           255u, 8u, "right", "left");
  LOG_INFO("string: \"%s\", empty \"%s\", escapes \"%s\"", "hello", "", "tab\there\nnew\\line\x01");
  LOG_INFO("utf-8: %s", "caf\xc3\xa9 \xe2\x9c\x93");
  LOG_INFO("long string is truncated to the frame: %s",
           "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
           "0123456789abcdef0123456789abcdef");
  LOG_INFO("pointer %p, size_t %zu, percent 100%%", (void *)&heartbeat, sizeof(bytes));
  LOG_INFO("sym: function {sym}%p, variable {sym}%p, unknown {sym}%p", (void *)log_selftest,
           (void *)&heartbeat, (void *)0x12345678);
  LOG_INFO("enum: {enum:app_state_t}%d {enum:sensor_mode}%d, unknown value {enum:app_state_t}%d",
           app_state, MODE_FAST, 7);
  LOG_INFO("14 args: %d %d %d %d %d %d %d %d %d %d %d %d %d %d", 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
           12, 13, 14);
  LOG_MEM_INFO("bytes", bytes, sizeof(bytes));
  sensor_dump();
}

// Bursts shorter than this also time the port's own buffering, past the log ring: a BLE stack
// queues tens of kB of notifications, so the reported rate is too high (unset: no such buffering).
#if defined(BURST_MIN_LOGS) && BURST_MIN_LOGS > 0
#define STR_(x) #x
#define STR(x) STR_(x)
#define BURST_SHORT(n) ((n) < BURST_MIN_LOGS ? " (short burst: the rate includes buffering; use burst " STR(BURST_MIN_LOGS) " or more)" : "")
#else
#define BURST_SHORT(n) ""
#endif

static void burst(int port, unsigned n) {
  log_stats_t before, after;
  log_get_stats(&before);
  uint32_t t0 = board_ms();
  for (unsigned i = 0; i < n; i++) {
    // Wait for room rather than drop: the buffer drains at ~100 kB/s.
    while (log_tx_avail() < 64) log_poll();
    LOG_INFO("burst %u/%u t=%lu", i + 1, n, (unsigned long)board_ms());
  }
  // Stop the clock when the last byte has gone out, not when there is room again: the 8 KiB
  // buffer would still hold several kB.
  uint32_t t1 = board_ms();
  while (!log_tx_flushed() && board_ms() - t1 < 2000) log_poll();
  bool flushed = log_tx_flushed();
  uint32_t ms = board_ms() - t0;
  log_get_stats(&after);
  uint32_t bytes = after.tx_bytes - before.tx_bytes;
  uint32_t tenths = n ? (uint32_t)((uint64_t)bytes * 10 / n) : 0;
  reply(port, "burst of %u logs: %lu bytes (%lu.%lu per log) in %lu ms = %lu B/s, %lu dropped%s%s\r\n", n,
        (unsigned long)bytes, (unsigned long)(tenths / 10), (unsigned long)(tenths % 10), (unsigned long)ms,
        (unsigned long)(ms ? (uint64_t)bytes * 1000 / ms : 0),
        (unsigned long)(after.tx_dropped - before.tx_dropped), flushed ? "" : " (not flushed: no host?)",
        BURST_SHORT(n));
}

// One frame of n bytes: "<n>:" then a repeating pattern, ending in "|\r\n".
static void big(int port, size_t n) {
  uint8_t *b = frame_buf;
  if (n < 16 || n > LOG_MAX_PACKET_SIZE) {
    reply(port, "big: n must be 16..%d\r\n", LOG_MAX_PACKET_SIZE);
    return;
  }
  int h = snprintf((char *)b, sizeof(frame_buf), "%u:", (unsigned)n);
  for (size_t i = (size_t)h; i < n - 3; i++) b[i] = (uint8_t)('a' + i % 26);
  memcpy(b + n - 3, "|\r\n", 3);
  // Bounded like reply(): this runs without log_poll(), so RX is not drained meanwhile. Without
  // room by then (no host), log_tx() drops the frame and counts it.
  uint32_t t0 = board_ms();
  while (log_tx_avail() < n && board_ms() - t0 < 200) {
  }
  log_tx((uint8_t)port, b, n);
  LOG_INFO("big: sent %u bytes on port %d, crc32c %08lx", (unsigned)n, port,
           (unsigned long)crc32c_update(CRC32C_INIT, b, n));
}

// Port 5: echo, and log the size and CRC so both directions can be checked.
static void on_echo(const uint8_t *rx, size_t n, void *ctx) {
  LOG_INFO("echo: received %u bytes, crc32c %08lx", (unsigned)n, (unsigned long)crc32c_update(CRC32C_INIT, rx, n));
  uint32_t t0 = board_ms();
  while (log_tx_avail() < n && board_ms() - t0 < 200) {
  }
  log_tx(ECHO_PORT, rx, n);
}

static void run_command(int port, char *c) {
  char *arg = strchr(c, ' ');
  if (arg) *arg++ = 0;
  app_state = APP_BUSY;
  if (strcmp(c, "") == 0) {
  } else if (strcmp(c, "help") == 0) {
    reply(port, "commands:\r\n"
                "  help        this list\r\n"
                "  info        resend device info (port 62), show app hash\r\n"
                "  test        log every argument kind\r\n"
                "  sh          semihost selftest (printf, files, time)\r\n"
                "  input       read a line from the host's stdin (semihost)\r\n"
                "  burst [n]   n logs as fast as the link allows (default 1000)\r\n"
                "  big [n]     one port-61 frame of n bytes (default the maximum, 1443)\r\n"
                "  stats       link counters\r\n"
                "  link        the link in use (uart, cdc, hid)\r\n"
                "  led on|off  user LED (stops blinking)\r\n"
                "  fatal       LOG_FATAL (resets the board)\r\n");
  } else if (strcmp(c, "info") == 0) {
    size_t n;
    const uint8_t *h = log_app_hash(&n);
    reply(port, "board %s, app_hash %02x%02x%02x%02x..., built %s %s\r\n", BOARD_NAME, h[0], h[1], h[2],
          h[3], __DATE__, __TIME__);
    log_send_device_info();
  } else if (strcmp(c, "test") == 0) {
    log_selftest();
    reply(port, "logged the selftest\r\n");
  } else if (strcmp(c, "sh") == 0) {
    sh_selftest();
    reply(port, "semihost selftest done (see the logs)\r\n");
  } else if (strcmp(c, "input") == 0) {
    reply(port, "type a line for the device's stdin:\r\n");
    char b[64];
    if (fgets(b, sizeof(b), stdin) != NULL) {
      b[strcspn(b, "\r\n")] = 0;
      LOG_INFO("stdin: \"%s\"", b);
      reply(port, "got \"%s\"\r\n", b);
    } else {
      reply(port, "stdin: EOF or error\r\n");
      clearerr(stdin);
    }
  } else if (strcmp(c, "burst") == 0) {
    unsigned n = arg ? (unsigned)strtoul(arg, NULL, 0) : 1000;
    burst(port, n);
  } else if (strcmp(c, "big") == 0) {
    big(port, arg ? (size_t)strtoul(arg, NULL, 0) : LOG_MAX_PACKET_SIZE);
  } else if (strcmp(c, "stats") == 0) {
    log_stats_t s;
    log_get_stats(&s);
    reply(port, "tx_bytes %lu tx_dropped %lu rx_frames %lu rx_errors %lu rx_uart_errors %lu rx_no_handler %lu\r\n",
          (unsigned long)s.tx_bytes, (unsigned long)s.tx_dropped, (unsigned long)s.rx_frames, (unsigned long)s.rx_errors,
          (unsigned long)s.rx_uart_errors, (unsigned long)s.rx_no_handler);
  } else if (strcmp(c, "link") == 0) {
    reply(port, "link: %s\r\n", board_link());
  } else if (strcmp(c, "led") == 0) {
    board_led(arg && strcmp(arg, "on") == 0);
    blink = false;
  } else if (strcmp(c, "fatal") == 0) {
    LOG_FATAL("fatal requested from port %d", port);
  } else {
    reply(port, "unknown command \"%s\" (try help)\r\n", c);
  }
  app_state = APP_RUNNING;
  if (port == TERM_PORT) reply(port, "> ");
}

int main(void) {
  log_pre_init();
  board_init();
  log_init(BOARD_NAME, "app");
  static char outbuf[128];
  setvbuf(stdout, outbuf, _IOLBF, sizeof(outbuf));

  const uint8_t *uid = board_uid();
  LOG_INFO("boot: %s, uid %02x%02x%02x%02x%02x%02x%02x%02x%02x%02x%02x%02x", BOARD_NAME, uid[11], uid[10],
           uid[9], uid[8], uid[7], uid[6], uid[5], uid[4], uid[3], uid[2], uid[1], uid[0]);
  log_notify(TERM_PORT, on_term, (void *)TERM_PORT);
  log_notify(CMD_PORT, on_term, (void *)CMD_PORT);
  log_notify(ECHO_PORT, on_echo, NULL);
  sensor_init(MODE_SLOW);
  log_selftest();
  app_state = APP_RUNNING;

  uint32_t next_beat = board_ms() + 1000, next_blink = board_ms() + 250;
  bool was_host = false;
  while (1) {
    log_poll();
    bool host;
    log_is_host_ready(&host);
    if (host != was_host) {
      LOG_INFO("host %s (%s)", host ? "connected" : "gone", board_link());
      if (host) reply(TERM_PORT, "\r\nucLog test firmware on %s - type help\r\n> ", BOARD_NAME);
      was_host = host;
    }
    if (cmd_tail != cmd_head) {
      run_command(cmds[cmd_tail % CMD_QUEUE].port, cmds[cmd_tail % CMD_QUEUE].text);
      cmd_tail++;
    }
    uint32_t now = board_ms();
    if ((int32_t)(now - next_blink) >= 0) {
      next_blink = now + (host ? 250 : 1000); // fast blink while a host is connected
      if (blink) board_led_toggle();
    }
    if ((int32_t)(now - next_beat) >= 0) {
      next_beat += 1000;
      if ((int32_t)(now - next_beat) >= 0) next_beat = now + 1000; // blocked (e.g. stdin): resync
      float t = sensor_read(now);
      heartbeat++;
      LOG_INFO("heartbeat %lu, uptime %lu.%03lu s, temp %.2f C, sensor {enum:sensor_state_t}%d",
               (unsigned long)heartbeat, (unsigned long)(now / 1000), (unsigned long)(now % 1000), t,
               sensor_state());
    }
    __asm volatile("wfi"); // SysTick or the UART wakes us
  }
}
