// © 2025 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// ucLog device side: log encoding (lib/log.c) and the frame server (lib/logserver.c) from
// github.com/UnitCircleInc/uclog, reduced to a polled, RTOS-free form.
//
// TX: frames are COBS-encoded straight into a ring buffer that the UART TX interrupt drains.
// While the host is absent (nothing received for 2 s) the ring is not drained, so boot logs are
// kept until a host connects; when it is full, new frames are dropped and counted. The first
// byte received resumes TX, and the port-62 device info goes out before the buffered frames.
//
// RX: the UART RX interrupt fills a second ring; log_poll() splits it into frames.

#include "log.h"

#include <stdarg.h>
#include <string.h>

#define DEVICE_INFO_PORT (62)
#define PULSE_MS (500)  // send something at least this often while the host is present
#define HOST_TIMEOUT_MS (2000)
#define MAX_FRAME (4096) // largest encoded frame accepted (protocol.md §2.2)

#define TX_MASK (LOG_TX_BUF_SIZE - 1)
#define RX_MASK (LOG_RX_BUF_SIZE - 1)
_Static_assert((LOG_TX_BUF_SIZE & TX_MASK) == 0, "LOG_TX_BUF_SIZE must be a power of two");
_Static_assert((LOG_RX_BUF_SIZE & RX_MASK) == 0, "LOG_RX_BUF_SIZE must be a power of two");
_Static_assert(LOG_MAX_LOG_SIZE >= 16 && LOG_MAX_LOG_SIZE <= 250, "LOG_MAX_LOG_SIZE");

// Filled in after linking by tools/apphash.py (SHA-512 of the image with this field zeroed).
static const __attribute__((used, section(".apphash"))) uint8_t app_hash[LOG_APP_HASH_SIZE];

static struct {
  // TX ring. Writers reserve [tx_reserve, +size) with interrupts off, encode with them on, and
  // publish: tx_head catches up with tx_reserve when the outermost writer finishes (a log from
  // an interrupt can nest inside another). tx_tail belongs to the TX interrupt.
  uint8_t tx[LOG_TX_BUF_SIZE];
  volatile uint32_t tx_head, tx_reserve, tx_tail;
  volatile uint32_t tx_nest;
  volatile bool host;          // host present: drain the TX ring
  volatile bool tx_boundary;   // the last byte sent ended a frame
  volatile uint32_t prefix_i;  // device info bytes sent (prefix_n = none pending)
  uint8_t prefix[160];
  uint32_t prefix_n;
  volatile uint32_t last_rx_ms, last_tx_ms;
  volatile uint32_t tx_dropped, tx_dropped_reported;
  bool panic;

  // RX ring; head is written by the RX interrupt, tail by log_poll().
  uint8_t rx[LOG_RX_BUF_SIZE];
  volatile uint32_t rx_head, rx_tail;
  volatile uint32_t rx_overrun;
  uint8_t frame[MAX_FRAME];
  size_t frame_n;
  bool synced, oversize, in_dispatch;
  uint32_t rx_frames, rx_errors, rx_no_handler;

  log_cb_t *handlers[64];
  void *contexts[64];

  // log_rx() waiting state.
  int wait_port;
  uint8_t *wait_data;
  size_t wait_cap, wait_n;
  bool wait_done;
} s;

// ---------------------------------------------------------------------------------------------
// TX

static uint32_t tx_free(void) {
  return LOG_TX_BUF_SIZE - (s.tx_reserve - s.tx_tail);
}

// Size of `00 COBS(hdr ‖ data) 00`, matching cobs_put().
static size_t cobs_size(const uint8_t *hdr, size_t hn, const uint8_t *data, size_t n) {
  size_t total = hn + n, len = 3; // delimiters + first code byte
  uint8_t code = 1;
  for (size_t i = 0; i < total; i++) {
    uint8_t b = i < hn ? hdr[i] : data[i - hn];
    if (b == 0) {
      len++;
      code = 1;
    } else {
      len++;
      if (++code == 0xFF && i + 1 < total) {
        len++;
        code = 1;
      }
    }
  }
  return len;
}

// Write `00 COBS(hdr ‖ data) 00` to buf[w & mask...]; returns the end position. A full 0xFF
// block that ends exactly at the end of the payload gets no trailing code byte (protocol.md §2.1).
static uint32_t cobs_put(uint8_t *buf, uint32_t mask, uint32_t w, const uint8_t *hdr, size_t hn,
                         const uint8_t *data, size_t n) {
  size_t total = hn + n;
  buf[w++ & mask] = 0;
  uint32_t code_at = w++;
  uint8_t code = 1;
  bool open = true;
  for (size_t i = 0; i < total; i++) {
    uint8_t b = i < hn ? hdr[i] : data[i - hn];
    if (b == 0) {
      buf[code_at & mask] = code;
      code_at = w++;
      code = 1;
    } else {
      buf[w++ & mask] = b;
      if (++code == 0xFF) {
        buf[code_at & mask] = code;
        open = i + 1 < total;
        if (open) {
          code_at = w++;
          code = 1;
        }
      }
    }
  }
  if (open) buf[code_at & mask] = code;
  buf[w++ & mask] = 0;
  return w;
}

static void kick(void) {
  if (s.host && !s.panic) log_port_tx_start();
}

// Interrupts are off only to reserve and to publish, never for the encoding: at 1 Mbaud a byte
// arrives every 10 us and the UART has a single receive register.
static bool tx_frame(const uint8_t *hdr, size_t hn, const uint8_t *data, size_t n) {
  uint32_t size = (uint32_t)cobs_size(hdr, hn, data, n);
  uint32_t key = log_port_lock();
  if (tx_free() < size) {
    s.tx_dropped++;
    log_port_unlock(key);
    return false;
  }
  uint32_t w = s.tx_reserve;
  s.tx_reserve = w + size;
  s.tx_nest++;
  log_port_unlock(key);

  cobs_put(s.tx, TX_MASK, w, hdr, hn, data, n);

  key = log_port_lock();
  // Publish when the outermost writer is done (or at once when panicking: nothing will resume).
  if (--s.tx_nest == 0 || s.panic) s.tx_head = s.tx_reserve;
  s.last_tx_ms = log_port_ms();
  log_port_unlock(key);
  kick();
  return true;
}

int log_uart_tx_next(void) {
  // Device info first, but only between frames.
  if (s.prefix_i < s.prefix_n && s.tx_boundary) {
    uint8_t b = s.prefix[s.prefix_i++];
    if (s.prefix_i == s.prefix_n) s.tx_boundary = true;
    return b;
  }
  // When the host goes away, finish the frame in progress and stop.
  if (s.tx_tail == s.tx_head || (!s.host && s.tx_boundary)) return -1;
  uint8_t b = s.tx[s.tx_tail & TX_MASK];
  s.tx_tail = s.tx_tail + 1;
  // A 00 ends a frame unless the next byte continues it (00 is never inside a frame).
  s.tx_boundary = b == 0;
  return b;
}

bool log_tx(uint8_t port, const uint8_t *data, size_t n) {
  if (port > 63) LOG_FATAL("invalid port %d", port);
  if (n > LOG_MAX_PACKET_SIZE) LOG_FATAL("tx message too long %zu", n);
  uint8_t hdr = (uint8_t)((port << 2) | 3);
  return tx_frame(&hdr, 1, data, n);
}

size_t log_tx_avail(void) {
  uint32_t f = tx_free();
  return f > LOG_MAX_PACKET_SIZE ? LOG_MAX_PACKET_SIZE : f;
}

// ---------------------------------------------------------------------------------------------
// Logging

// The format string's address with the frame type in its low two bits (they are always zero:
// the strings are 4-byte aligned).
static void put_addr(uint8_t *b, const void *p, uint8_t type) {
  uint32_t a = (uint32_t)(uintptr_t)p;
  a = (a & ~3u) | type;
  memcpy(b, &a, 4);
}

void log_log1_(const char *prefix) {
  uint8_t b[4];
  put_addr(b, prefix, 0);
  tx_frame(b, 4, NULL, 0);
}

void log_logn_(const char *kinds, const char *prefix, ...) {
  uint8_t b[LOG_MAX_LOG_SIZE];
  size_t n = 4;
  put_addr(b, prefix, 0);

  va_list ap;
  va_start(ap, prefix);
  for (; *kinds != '\0'; kinds++) {
    switch (*kinds) {
    case '0': {
      unsigned int v = va_arg(ap, unsigned int);
      if (n + 4 > sizeof(b)) goto done;
      memcpy(b + n, &v, 4);
      n += 4;
      break;
    }
    case '1': {
      unsigned long long v = va_arg(ap, unsigned long long);
      if (n + 8 > sizeof(b)) goto done;
      memcpy(b + n, &v, 8);
      n += 8;
      break;
    }
    case '2': {
      double v = va_arg(ap, double);
      if (n + 8 > sizeof(b)) goto done;
      memcpy(b + n, &v, 8);
      n += 8;
      break;
    }
    case '3': { // long double is a double on ARM EABI; sent as one
      double v = (double)va_arg(ap, long double);
      if (n + 8 > sizeof(b)) goto done;
      memcpy(b + n, &v, 8);
      n += 8;
      break;
    }
    case '4': { // NUL-terminated; truncated to fit the frame
      const char *p = va_arg(ap, const char *);
      if (n + 1 > sizeof(b)) goto done;
      if (p == NULL) p = "(null)";
      size_t room = sizeof(b) - n - 1;
      const char *z = memchr(p, 0, room);
      size_t sn = z ? (size_t)(z - p) : room;
      memcpy(b + n, p, sn);
      n += sn;
      b[n++] = 0;
      break;
    }
    default: { // '5' pointer
      const void *p = va_arg(ap, const void *);
      uint32_t v = (uint32_t)(uintptr_t)p;
      if (n + 4 > sizeof(b)) goto done;
      memcpy(b + n, &v, 4);
      n += 4;
      break;
    }
    }
  }
done:
  va_end(ap);
  tx_frame(b, n, NULL, 0);
}

void log_mem_(const char *prefix, const void *data, size_t n) {
  uint8_t b[8];
  if (n > LOG_MAX_LOG_SIZE - 8) n = LOG_MAX_LOG_SIZE - 8;
  put_addr(b, prefix, 1);
  uint32_t a = (uint32_t)(uintptr_t)data;
  memcpy(b + 4, &a, 4);
  tx_frame(b, 8, data, n);
}

// ---------------------------------------------------------------------------------------------
// Panic: from here on, frames are written synchronously with interrupts off.

static void flush_polled(void) {
  int c;
  s.host = true;
  while ((c = log_uart_tx_next()) >= 0) log_port_tx_poll((uint8_t)c);
}

void log_panic_(void) {
  (void)log_port_lock();
  s.panic = true;
  flush_polled();
}

__attribute__((weak)) void log_fatal(void) {}

__attribute__((noreturn)) void log_fatal_(void) {
  flush_polled();
  log_fatal();
  log_port_fatal();
}

// ---------------------------------------------------------------------------------------------
// Device info (port 62): CBOR {"app_hash": bytes(64), "board": text, "image_type": text}

static size_t cbor_head(uint8_t *b, uint8_t major, size_t n) {
  if (n < 24) {
    b[0] = (uint8_t)((major << 5) | n);
    return 1;
  }
  b[0] = (uint8_t)((major << 5) | 24);
  b[1] = (uint8_t)n;
  return 2;
}

static size_t cbor_text(uint8_t *b, const char *t) {
  size_t n = strlen(t);
  size_t h = cbor_head(b, 3, n);
  memcpy(b + h, t, n);
  return h + n;
}

static size_t device_info(uint8_t *b, const char *board, const char *image_type) {
  size_t n = cbor_head(b, 5, 3);
  n += cbor_text(b + n, "app_hash");
  n += cbor_head(b + n, 2, LOG_APP_HASH_SIZE);
  memcpy(b + n, app_hash, LOG_APP_HASH_SIZE);
  n += LOG_APP_HASH_SIZE;
  n += cbor_text(b + n, "board");
  n += cbor_text(b + n, board);
  n += cbor_text(b + n, "image_type");
  n += cbor_text(b + n, image_type);
  return n;
}

static uint8_t info[128];
static size_t info_n;

void log_send_device_info(void) {
  log_tx(DEVICE_INFO_PORT, info, info_n);
}

const uint8_t *log_app_hash(size_t *n) {
  if (n != NULL) *n = LOG_APP_HASH_SIZE;
  return app_hash;
}

void log_init(const char *board, const char *image_type) {
  memset(&s, 0, sizeof(s));
  s.wait_port = -1;
  s.tx_boundary = true;

  // Pre-encode the device info frame that precedes the buffered data whenever a host connects.
  if (strlen(board) > 23) board = "?";
  if (strlen(image_type) > 23) image_type = "?";
  info_n = device_info(info, board, image_type);
  uint8_t hdr = (DEVICE_INFO_PORT << 2) | 3;
  _Static_assert(sizeof(info) + 1 + 3 + 1 <= sizeof(s.prefix), "prefix too small");
  s.prefix_n = cobs_put(s.prefix, 0xFFFFFFFFu, 0, &hdr, 1, info, info_n);
  s.prefix_i = s.prefix_n;

  LOG_INFO("log-init");
}

// ---------------------------------------------------------------------------------------------
// Liveness

void log_uart_rx(uint8_t b, bool error) {
  if (error) s.rx_overrun++;
  uint32_t h = s.rx_head;
  if (h - s.rx_tail < LOG_RX_BUF_SIZE) {
    s.rx[h & RX_MASK] = b;
    s.rx_head = h + 1;
  } else {
    s.rx_overrun++;
  }
  s.last_rx_ms = log_port_ms();
  if (!s.host) {
    // Host (re)connected: device info, then whatever is buffered.
    s.host = true;
    s.prefix_i = 0;
    kick();
  }
}

void log_tick(uint32_t now) {
  if (!s.host) return;
  if (now - s.last_rx_ms > HOST_TIMEOUT_MS) {
    s.host = false;
    return;
  }
  if (now - s.last_tx_ms >= PULSE_MS) {
    uint32_t key = log_port_lock();
    if (s.tx_head == s.tx_tail && s.tx_reserve == s.tx_head) { // idle, no writer in progress
      s.tx[s.tx_head & TX_MASK] = 0;
      s.tx_reserve = s.tx_head = s.tx_head + 1;
    }
    s.last_tx_ms = now;
    log_port_unlock(key);
    kick();
  }
}

int log_is_host_ready(bool *host_ready) {
  *host_ready = s.host;
  return 0;
}

void log_get_stats(log_stats_t *st) {
  st->tx_dropped = s.tx_dropped;
  st->rx_overrun = s.rx_overrun;
  st->rx_frames = s.rx_frames;
  st->rx_errors = s.rx_errors;
  st->rx_no_handler = s.rx_no_handler;
}

// ---------------------------------------------------------------------------------------------
// RX

// In-place COBS decode; returns the payload length or -1. The trailing zero of a final
// non-0xFF block is implicit.
static int cobs_dec(uint8_t *b, size_t n) {
  size_t r = 0, w = 0;
  while (r < n) {
    uint8_t code = b[r++];
    if (code == 0) return -1;
    for (uint8_t i = 1; i < code; i++) {
      if (r >= n) return -1;
      b[w++] = b[r++];
    }
    if (code != 0xFF && r < n) b[w++] = 0;
  }
  return (int)w;
}

static void dispatch(uint8_t *p, size_t n) {
  if (n == 0) return; // pulse (encoded as 01)
  s.rx_frames++;
  uint8_t type = p[0] & 3, port = p[0] >> 2;
  if (type != 3) {
    s.rx_errors++;
    LOG_WARN("unexpected frame type %d from host", type);
    return;
  }
  if (port == s.wait_port && !s.wait_done) {
    size_t m = n - 1 < s.wait_cap ? n - 1 : s.wait_cap;
    memcpy(s.wait_data, p + 1, m);
    s.wait_n = n - 1;
    s.wait_done = true;
  } else if (s.handlers[port] != NULL) {
    s.in_dispatch = true;
    s.handlers[port](p + 1, n - 1, s.contexts[port]);
    s.in_dispatch = false;
  } else {
    s.rx_no_handler++;
    LOG_WARN("no handler for port %d (%zu bytes)", port, n - 1);
  }
}

void log_poll(void) {
  if (s.in_dispatch) return; // called from a handler: the frame buffer is in use
  if (s.tx_dropped != s.tx_dropped_reported && tx_free() > LOG_MAX_LOG_SIZE * 2) {
    uint32_t d = s.tx_dropped;
    LOG_WARN("TX buffer full: %lu frames dropped", (unsigned long)(d - s.tx_dropped_reported));
    s.tx_dropped_reported = d;
  }
  while (s.rx_tail != s.rx_head) {
    uint8_t b = s.rx[s.rx_tail & RX_MASK];
    s.rx_tail = s.rx_tail + 1;
    if (b != 0) {
      // Bytes before the first 00 belong to a frame whose start was missed.
      if (!s.synced) continue;
      if (s.frame_n < sizeof(s.frame)) {
        s.frame[s.frame_n++] = b;
      } else {
        s.oversize = true;
      }
      continue;
    }
    if (s.synced && s.frame_n > 0) {
      int n = s.oversize ? -1 : cobs_dec(s.frame, s.frame_n);
      if (n < 0) {
        s.rx_errors++;
        LOG_WARN("bad frame from host (%zu bytes%s)", s.frame_n, s.oversize ? ", too long" : "");
      } else {
        dispatch(s.frame, (size_t)n);
      }
    }
    s.synced = true;
    s.oversize = false;
    s.frame_n = 0;
    // A handler may have started a log_rx() wait; let it see its reply first.
    if (s.wait_done) return;
  }
}

void log_notify(uint8_t port, log_cb_t *cb, void *ctx) {
  if (port > 63) LOG_FATAL("port out of range: %d", port);
  s.contexts[port] = ctx;
  s.handlers[port] = cb;
}

size_t log_rx(uint8_t port, uint8_t *data, size_t n, uint32_t timeout_ms) {
  if (port > 63) LOG_FATAL("invalid port %d", port);
  if (s.in_dispatch || s.wait_port >= 0) {
    LOG_ERROR("log_rx(%d) called from a handler or nested", port);
    return LOG_RX_ERROR;
  }
  s.wait_data = data;
  s.wait_cap = n;
  s.wait_done = false;
  s.wait_port = port;
  uint32_t start = log_port_ms();
  size_t r = LOG_RX_ERROR;
  while (true) {
    log_poll();
    if (s.wait_done) {
      r = s.wait_n;
      break;
    }
    if (!s.host) break;
    if (timeout_ms != LOG_RX_FOREVER && log_port_ms() - start >= timeout_ms) break;
  }
  s.wait_port = -1;
  return r;
}
