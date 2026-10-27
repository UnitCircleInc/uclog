// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// log_port.h in memory, for the host test: TX drains straight into `host_tx` (unless paused);
// RX is a buffer the driver fills.

#include "host_port.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

uint8_t host_tx[1 << 20];
size_t host_tx_n;
uint32_t host_ms;

static cb_t *tx_cb;
static bool paused;
static uint8_t rx[1 << 16];
static size_t rx_r, rx_w;
static uint32_t mark;

void host_rx_feed(const uint8_t *b, size_t n) {
  if (rx_r == rx_w) rx_r = rx_w = 0;
  memcpy(rx + rx_w, b, n);
  rx_w += n;
  mark++;
}

static void out(const uint8_t *b, size_t n) {
  memcpy(host_tx + host_tx_n, b, n);
  host_tx_n += n;
}

uint32_t log_port_lock(void) {
  return 0;
}
void log_port_unlock(uint32_t key) {
  (void)key;
}
uint32_t log_port_ms(void) {
  return host_ms;
}
void log_port_init(cb_t *cb) {
  tx_cb = cb;
}

void log_port_tx_schedule(const uint8_t *prefix, size_t n) {
  if (paused) return;
  if (prefix != NULL) out(prefix, n);
  while (cb_peek_avail(tx_cb) > 0) {
    size_t k = cb_peek_avail(tx_cb);
    out(cb_peek(tx_cb), k);
    cb_skip(tx_cb, k);
  }
}

void log_port_tx_pause(bool pause) {
  paused = pause;
}
bool log_port_tx_idle(void) {
  return true;
}
void log_port_panic(void) {}
size_t log_port_rx_avail(void) {
  return rx_w - rx_r;
}
const uint8_t *log_port_rx_peek(void) {
  return rx + rx_r;
}
void log_port_rx_skip(size_t n) {
  rx_r += n;
}
uint32_t log_port_rx_mark(void) {
  return mark;
}
uint32_t log_port_rx_check(void) {
  return 0;
}
__attribute__((noreturn)) void log_port_fatal(void) {
  fflush(stdout);
  abort();
}
