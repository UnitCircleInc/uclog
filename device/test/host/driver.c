// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// The device library on the host, driven line by line from stdin (test_vectors.py):
//   init                 log_pre_init(), log_init("host_test", "app")
//   ms N                 the clock is N ms; log_tick(N)
//   rx HEX               bytes from the host; log_poll()
//   tx PORT HEX          log_tx(PORT, HEX)
//   logs                 the log calls below, in order
//   notify PORT          a handler on PORT that prints "frame PORT HEX"
//   out                  prints "out HEX": what the port sent since the last `out`
//   stats                prints "stats tx_dropped rx_frames rx_errors rx_no_handler"
//   host                 prints "host 0|1"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "host_port.h"

static size_t unhex(const char *s, uint8_t *b) {
  size_t n = 0;
  while (s[0] && s[1] && s[0] != '\n') {
    unsigned v;
    sscanf(s, "%2x", &v);
    b[n++] = (uint8_t)v;
    s += 2;
  }
  return n;
}

static void hex(const char *tag, const uint8_t *b, size_t n) {
  printf("%s ", tag);
  for (size_t i = 0; i < n; i++) printf("%02x", b[i]);
  printf("\n");
}

static void on_frame(const uint8_t *rx, size_t n, void *ctx) {
  char tag[32];
  snprintf(tag, sizeof(tag), "frame %d", (int)(intptr_t)ctx);
  hex(tag, rx, n);
}

typedef enum { MODE_A, MODE_B = 5 } mode_t_;

// test_vectors.py expects these arguments, in this order (LOGS there).
static void logs(void) {
  static const uint8_t mem[20] = {0, 1, 2, 0x7f, 0x80, 0xff, 'u', 'c', 'L', 'o', 'g', 0, 0xde, 0xad, 0xbe, 0xef};
  LOG_INFO("no arguments");
  LOG_INFO("int %d unsigned %u hex %x", -123456, 4000000000u, 0xbeefu);
  LOG_INFO("small %d %u %d %d", (short)-2, (unsigned char)250, (_Bool)1, 'A');
  LOG_INFO("int64 %lld %llu", -9000000000000000000ll, 18000000000000000000ull);
  LOG_INFO("float %f double %f", 1.5f, 3.14159265358979);
  LOG_INFO("long double %Lf", (long double)2.5);
  LOG_INFO("string %s empty %s", "hello", "");
  LOG_INFO("pointer %p", (void *)(uintptr_t)0x2000abcdu);
  LOG_INFO("{enum:mode_t_}%d", MODE_B);
  LOG_INFO("14: %d %d %d %d %d %d %d %d %d %d %d %d %d %d", 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14);
  LOG_INFO("long %s", "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
  LOG_WARN("warn");
  LOG_ERROR("error");
  LOG_DEBUG("debug");
  LOG_MEM_INFO("bytes", mem, sizeof(mem));
}

int main(void) {
  static char line[8192];
  static uint8_t b[4096];
  setvbuf(stdout, NULL, _IOLBF, 0);
  while (fgets(line, sizeof(line), stdin)) {
    if (strncmp(line, "init", 4) == 0) {
      log_pre_init();
      log_init("host_test", "app");
    } else if (strncmp(line, "ms ", 3) == 0) {
      host_ms = (uint32_t)strtoul(line + 3, NULL, 10);
      log_tick(host_ms);
    } else if (strncmp(line, "rx ", 3) == 0) {
      size_t n = unhex(line + 3, b);
      host_rx_feed(b, n);
      log_tick(host_ms);
      log_poll();
    } else if (strncmp(line, "tx ", 3) == 0) {
      char *p;
      unsigned port = (unsigned)strtoul(line + 3, &p, 10);
      while (*p == ' ') p++;
      log_tx((uint8_t)port, b, unhex(p, b));
    } else if (strncmp(line, "logs", 4) == 0) {
      logs();
    } else if (strncmp(line, "notify ", 7) == 0) {
      int port = atoi(line + 7);
      log_notify((uint8_t)port, on_frame, (void *)(intptr_t)port);
    } else if (strncmp(line, "out", 3) == 0) {
      hex("out", host_tx, host_tx_n);
      host_tx_n = 0;
    } else if (strncmp(line, "stats", 5) == 0) {
      log_stats_t s;
      log_get_stats(&s);
      printf("stats %u %u %u %u\n", s.tx_dropped, s.rx_frames, s.rx_errors, s.rx_no_handler);
    } else if (strncmp(line, "host", 4) == 0) {
      bool h;
      log_is_host_ready(&h);
      printf("host %d\n", h);
    }
  }
  return 0;
}
