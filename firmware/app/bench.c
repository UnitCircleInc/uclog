// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// The `bench` command (plan 08 §6): for each log statement of a fixed set, the bytes it puts on
// the link and the CPU cycles the call takes, beside the same message formatted as text by
// snprintf. One line per statement on the terminal; firmware/bench.py collects them as JSON for
// the docs' performance page.
//
//   bench start board=<name> hz=<cpu Hz> n=<calls timed per statement>
//   bench <id> wire=<bytes> text=<bytes> log_min=<cycles> log_avg=<cycles> printf_min=<cycles> printf_avg=<cycles>
//   bench end
//
// wire: the frame as sent (COBS, frame header, delimiter), counted by the library as it queues
// it. text: snprintf's length, plus 2 for the line end a text log would send. Cycles are timed
// around each call, interrupts on (as in use): min is the call alone, avg includes what the
// interrupts took meanwhile.

#include "bench.h"

#include <stdarg.h>
#include <stdio.h>
#include <string.h>

#include "board.h"
#include "log.h"

#define BENCH_N 32

static void out(int port, const char *fmt, ...) __attribute__((format(printf, 2, 3)));
static void out(int port, const char *fmt, ...) {
  char b[160];
  va_list ap;
  va_start(ap, fmt);
  int n = vsnprintf(b, sizeof(b), fmt, ap);
  va_end(ap);
  if (n < 0) return;
  if ((size_t)n >= sizeof(b)) n = sizeof(b) - 1;
  while (log_tx_avail() < (size_t)n + 16) log_poll(); // the host is there: it drains
  log_tx((uint8_t)port, (const uint8_t *)b, (size_t)n);
}

// Room for a log, and nothing in flight from the last one: each timed call starts the same way.
static void settle(void) {
  uint32_t t0 = board_ms();
  while ((log_tx_avail() < 512 || !log_tx_flushed()) && board_ms() - t0 < 200) log_poll();
}

typedef struct {
  uint32_t wire, text, log_min, log_sum, printf_min, printf_sum;
} result_t;

static void report(int port, const char *id, const result_t *r) {
  out(port, "bench %s wire=%lu text=%lu log_min=%lu log_avg=%lu printf_min=%lu printf_avg=%lu\r\n", id,
      (unsigned long)r->wire, (unsigned long)r->text, (unsigned long)r->log_min,
      (unsigned long)(r->log_sum / BENCH_N), (unsigned long)r->printf_min,
      (unsigned long)(r->printf_sum / BENCH_N));
}

static char text[160];

// One statement: `fmt_args` is the format and arguments, as given to LOG_INFO and snprintf.
#define MEASURE(port_, id_, ...)                                                                 \
  do {                                                                                           \
    result_t r = {.wire = UINT32_MAX, .log_min = UINT32_MAX, .printf_min = UINT32_MAX};         \
    for (int i = 0; i < BENCH_N; i++) {                                                          \
      settle();                                                                                  \
      log_stats_t s0, s1;                                                                        \
      log_get_stats(&s0);                                                                        \
      uint32_t t0 = board_cycles();                                                              \
      LOG_INFO(__VA_ARGS__);                                                                     \
      uint32_t t1 = board_cycles();                                                              \
      log_get_stats(&s1);                                                                        \
      uint32_t wire = s1.tx_bytes - s0.tx_bytes;                                                 \
      if (wire < r.wire) r.wire = wire; /* min: a pulse may have been queued meanwhile */        \
      if (t1 - t0 < r.log_min) r.log_min = t1 - t0;                                              \
      r.log_sum += t1 - t0;                                                                      \
      t0 = board_cycles();                                                                       \
      int n = snprintf(text, sizeof(text), __VA_ARGS__);                                         \
      t1 = board_cycles();                                                                       \
      r.text = (uint32_t)n + 2;                                                                  \
      if (t1 - t0 < r.printf_min) r.printf_min = t1 - t0;                                        \
      r.printf_sum += t1 - t0;                                                                   \
    }                                                                                            \
    report(port_, id_, &r);                                                                      \
  } while (0)

void bench(int port) {
  volatile int count = 1234, x = -56, y = 789, z = 100000; // volatile: no constant folding
  volatile float temp = 23.5f;
  volatile double ratio = 0.618034;
  const char *volatile name = "sensor-1";
  out(port, "bench start board=%s hz=%lu n=%d\r\n", BOARD_NAME, (unsigned long)board_hz(), BENCH_N);
  MEASURE(port, "args0", "bench: no arguments");
  MEASURE(port, "int1", "bench: count %d", count);
  MEASURE(port, "int2", "bench: x=%d y=%d", x, y);
  MEASURE(port, "int4", "bench: %d %d %d %d", count, x, y, z);
  MEASURE(port, "float1", "bench: temp %.2f", temp);
  MEASURE(port, "double1", "bench: ratio %f", ratio);
  MEASURE(port, "str1", "bench: name %s", name);
  MEASURE(port, "mixed4", "bench: %s count=%d temp=%.1f ratio=%f", name, count, temp, ratio);
  out(port, "bench end\r\n");
}
