// © 2025 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// ucLog device side: log encoding (lib/log.c) and the frame server (lib/logserver.c) from
// github.com/UnitCircleInc/uclog, without Zephyr. COBS, CBOR and the ring buffer are the
// ucComm implementations (github.com/UnitCircleInc/ucComm).
//
// TX: frames are COBS-encoded on the stack and copied into the tx_cb ring buffer, which the port
// drains (DMA on the STM32L4 board). While the host is absent (nothing received for 2 s) the
// port is paused, so boot logs are kept until a host connects; when the ring is full, new frames
// are dropped and counted. When bytes arrive again, the port-62 device info is sent first, then
// the backlog. An empty port-62 frame from the host asks for the device info again (F7).
//
// RX: log_poll() reads the port's RX ring (DMA on the STM32L4 board), splits it into frames and
// dispatches them to the port handlers, or to a waiting log_rx().

#include "log.h"

#include <stdarg.h>
#include <string.h>

#include "cbor.h"
#include "cobs.h"

#define DEVICE_INFO_PORT (62)
#define PULSE_MS (500)   // send something at least this often while the host is present
#define HOST_TIMEOUT_MS (2000)
#define MAX_FRAME (LOG_MAX_FRAME_SIZE - 2) // largest encoded run accepted (protocol.md §2.2)

_Static_assert(COBS_ENC_SIZE(LOG_MAX_PACKET_SIZE + 1) + 2 == LOG_MAX_FRAME_SIZE, "frame size");
_Static_assert(LOG_MAX_LOG_SIZE >= 16 && LOG_MAX_LOG_SIZE <= 250, "LOG_MAX_LOG_SIZE");

// Filled in after linking by tools/apphash.py (SHA-512 of the image with this field zeroed).
static const __attribute__((used, section(".apphash"))) uint8_t app_hash[LOG_APP_HASH_SIZE];

static cb_t tx_cb;
static uint8_t tx_buf[LOG_TX_BUF_SIZE];

#define MAX_DEVICE_INFO_SIZE 192
static uint8_t device_info_tx_buf[COBS_ENC_SIZE(MAX_DEVICE_INFO_SIZE) + 2];
static size_t device_info_len;

static struct {
  volatile bool host;   // host present: TX runs
  bool started;         // log_init() done
  bool panic;
  uint32_t rx_mark;
  volatile uint32_t last_rx_ms, last_tx_ms;
  volatile uint32_t tx_dropped, tx_dropped_reported, tx_bytes;

  uint8_t frame[MAX_FRAME];
  size_t frame_n;
  bool synced, oversize, in_dispatch;
  uint32_t rx_frames, rx_errors, rx_no_handler, rx_uart_errors;

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

static void tx_buffer(const uint8_t *b, size_t n) {
  uint32_t key = log_port_lock();
  if (cb_write_avail(&tx_cb) >= n) {
    cb_write(&tx_cb, b, n);
    s.tx_bytes += n;
    s.last_tx_ms = log_port_ms();
  } else {
    s.tx_dropped++;
  }
  log_port_unlock(key);
  if (s.host && s.started) log_port_tx_schedule(NULL, 0);
}

// `00 COBS(payload) 00` into `out` (COBS_ENC_SIZE(n) + 2 bytes); returns the frame length. The same
// output as ucComm's cobs_enc() with the delimiters added, but the current block's code byte is a
// pointer and its length a register, not read back from memory for every byte.
static size_t frame(uint8_t *out, const uint8_t *in, size_t n) {
  const uint8_t *end = in + n;
  uint8_t *code = out + 1, *o = out + 2;
  uint8_t c = 1;
  out[0] = 0;
  while (in < end) {
    uint8_t v = *in++;
    if (v != 0) {
      *o++ = v;
      if (++c < 255) continue;
      if (in == end) break; // a full block at the end needs no empty block after it
    }
    *code = c;
    code = o++;
    c = 1;
  }
  *code = c;
  *o++ = 0;
  return (size_t)(o - out);
}

void log_tx(uint8_t port, const uint8_t *data, size_t n) {
  static uint8_t p[LOG_MAX_PACKET_SIZE + 1];
  static uint8_t b[LOG_MAX_FRAME_SIZE];
  if (n > LOG_MAX_PACKET_SIZE) LOG_FATAL("tx message too long %zu", n);
  if (63 < port) LOG_FATAL("invalid port %d", port);
  // Static buffers: hold the lock so a log_tx() from an interrupt cannot interleave. With DMA
  // reception, interrupt latency does not lose received bytes.
  uint32_t key = log_port_lock();
  p[0] = (uint8_t)((port << 2) | 3);
  memcpy(p + 1, data, n);
  size_t fn = frame(b, p, n + 1);
  tx_buffer(b, fn);
  log_port_unlock(key);
}

size_t log_tx_avail(void) {
  // Room for port data, after the frame overhead.
  uint32_t key = log_port_lock();
  size_t a = cb_write_avail(&tx_cb);
  log_port_unlock(key);
  size_t over = 3 + (a + 253) / 254;
  a = a > over ? a - over : 0;
  return a > LOG_MAX_PACKET_SIZE ? LOG_MAX_PACKET_SIZE : a;
}

bool log_tx_flushed(void) {
  uint32_t key = log_port_lock();
  bool done = cb_read_avail(&tx_cb) == 0 && log_port_tx_idle();
  log_port_unlock(key);
  return done;
}

// ---------------------------------------------------------------------------------------------
// Logging

// Little-endian stores at any alignment, inline: memcpy() is a call per argument on a Cortex-M0+.
static inline void put32(uint8_t *b, uint32_t v) {
  b[0] = (uint8_t)v;
  b[1] = (uint8_t)(v >> 8);
  b[2] = (uint8_t)(v >> 16);
  b[3] = (uint8_t)(v >> 24);
}

static inline void put64(uint8_t *b, uint64_t v) {
  put32(b, (uint32_t)v);
  put32(b + 4, (uint32_t)(v >> 32));
}

// The format string's address with the frame type in its low two bits (they are always zero:
// the strings are 4-byte aligned).
static void put_addr(uint8_t *b, const void *p, uint8_t type) {
  uint32_t a = (uint32_t)(uintptr_t)p;
  put32(b, (a & ~3u) | type);
}

static void tx_log(const uint8_t *payload, size_t n) {
  uint8_t b[COBS_ENC_SIZE(LOG_MAX_LOG_SIZE) + 2];
  tx_buffer(b, frame(b, payload, n));
}

void log_log1_(const char *prefix) {
  uint8_t b[4];
  put_addr(b, prefix, 0);
  tx_log(b, 4);
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
      put32(b + n, v);
      n += 4;
      break;
    }
    case '1': {
      unsigned long long v = va_arg(ap, unsigned long long);
      if (n + 8 > sizeof(b)) goto done;
      put64(b + n, v);
      n += 8;
      break;
    }
    case '2': {
      double v = va_arg(ap, double);
      uint64_t u;
      memcpy(&u, &v, 8); // between locals: no call
      if (n + 8 > sizeof(b)) goto done;
      put64(b + n, u);
      n += 8;
      break;
    }
    case '3': { // long double is a double on ARM EABI; sent as one
      double v = (double)va_arg(ap, long double);
      uint64_t u;
      memcpy(&u, &v, 8);
      if (n + 8 > sizeof(b)) goto done;
      put64(b + n, u);
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
      if (n + 4 > sizeof(b)) goto done;
      put32(b + n, (uint32_t)(uintptr_t)p);
      n += 4;
      break;
    }
    }
  }
done:
  va_end(ap);
  tx_log(b, n);
}

void log_mem_(const char *prefix, const void *data, size_t n) {
  uint8_t b[LOG_MAX_LOG_SIZE];
  if (n > LOG_MAX_LOG_SIZE - 8) n = LOG_MAX_LOG_SIZE - 8;
  put_addr(b, prefix, 1);
  uint32_t a = (uint32_t)(uintptr_t)data;
  memcpy(b + 4, &a, 4);
  memcpy(b + 8, data, n);
  tx_log(b, 8 + n);
}

// ---------------------------------------------------------------------------------------------
// Panic: from here on the port writes synchronously with interrupts off.

void log_panic_(void) {
  (void)log_port_lock();
  s.panic = true;
  s.host = true;
  if (s.started) {
    log_port_panic();
    log_port_tx_pause(false);
    log_port_tx_schedule(NULL, 0); // flush what is buffered
  }
}

__attribute__((weak)) void log_fatal(void) {}

__attribute__((noreturn)) void log_fatal_(void) {
  if (s.started) log_port_tx_schedule(NULL, 0);
  log_fatal();
  log_port_fatal();
}

// ---------------------------------------------------------------------------------------------
// Device info (port 62): CBOR {"app_hash": bytes(64), "image_type": text, "board": text,
// "uclog_version": text}

static void fill_device_info(const char *board, const char *image_type) {
  uint8_t p[MAX_DEVICE_INFO_SIZE];
  p[0] = (DEVICE_INFO_PORT << 2) | 3;
  cbor_stream_t cs;
  cbor_init(&cs, p + 1, sizeof(p) - 1);
  cbor_error_t err = cbor_pack(&cs, "{.app_hash:b,.image_type:s,.board:s,.uclog_version:s}", app_hash,
                               (size_t)LOG_APP_HASH_SIZE, image_type, board, UCLOG_DEVICE_VERSION);
  if (err != CBOR_ERROR_NONE) LOG_FATAL("CBOR pack error: %d", err);
  device_info_len = frame(device_info_tx_buf, p, 1 + cbor_read_avail(&cs));
}

void log_send_device_info(void) {
  tx_buffer(device_info_tx_buf, device_info_len);
}

const uint8_t *log_app_hash(size_t *n) {
  if (n != NULL) *n = LOG_APP_HASH_SIZE;
  return app_hash;
}

void log_pre_init(void) {
  memset(&s, 0, sizeof(s));
  s.wait_port = -1;
  cb_init(&tx_cb, tx_buf, sizeof(tx_buf));
  LOG_INFO("log-pre-init");
}

void log_init(const char *board, const char *image_type) {
  fill_device_info(board, image_type);
  log_port_init(&tx_cb);
  log_port_tx_pause(true); // until a host shows up
  s.rx_mark = log_port_rx_mark();
  s.started = true;
}

// ---------------------------------------------------------------------------------------------
// Liveness

// Host (re)connected: drop the rest of a frame cut off when the host left, then send the device
// info and the backlog.
static void host_resume(void) {
  uint32_t key = log_port_lock();
  if (log_port_tx_idle()) {
    while (cb_read_avail(&tx_cb) > 0 && *(const uint8_t *)cb_peek(&tx_cb) != 0) cb_skip(&tx_cb, 1);
  }
  s.host = true;
  log_port_tx_pause(false);
  log_port_tx_schedule(device_info_tx_buf, device_info_len);
  log_port_unlock(key);
}

void log_tick(uint32_t now) {
  if (!s.started || s.panic) return;
  // The timeout first: a board that hands the log to another link once the one in use has been
  // quiet for 2 s does so in the same tick, and the new link's bytes must count as a new host.
  if (s.host && now - s.last_rx_ms > HOST_TIMEOUT_MS) {
    s.host = false;
    log_port_tx_pause(true);
  }
  uint32_t mark = log_port_rx_mark();
  if (mark != s.rx_mark) {
    s.rx_mark = mark;
    s.last_rx_ms = now;
    if (!s.host) host_resume();
  }
  if (!s.host) return;
  if (now - s.last_tx_ms >= PULSE_MS && cb_read_avail(&tx_cb) == 0) {
    static const uint8_t pulse = 0;
    tx_buffer(&pulse, 1);
  }
}

int log_is_host_ready(bool *host_ready) {
  *host_ready = s.host;
  return 0;
}

void log_get_stats(log_stats_t *st) {
  st->tx_dropped = s.tx_dropped;
  st->tx_bytes = s.tx_bytes;
  st->rx_uart_errors = s.rx_uart_errors;
  st->rx_frames = s.rx_frames;
  st->rx_errors = s.rx_errors;
  st->rx_no_handler = s.rx_no_handler;
}

// ---------------------------------------------------------------------------------------------
// RX

static void dispatch(uint8_t *p, size_t n) {
  if (n == 0) return; // pulse (encoded as 01)
  s.rx_frames++;
  uint8_t type = p[0] & 3, port = p[0] >> 2;
  if (type != 3) {
    s.rx_errors++;
    LOG_WARN("unexpected frame type %d from host", type);
    return;
  }
  if (port == DEVICE_INFO_PORT) {
    // F7: an empty port-62 frame asks for the device info; other payloads are reserved.
    if (n == 1) {
      log_send_device_info();
    } else {
      s.rx_errors++;
      LOG_WARN("port %d: unexpected %zu-byte payload", port, n - 1);
    }
  } else if (port == s.wait_port && !s.wait_done) {
    size_t m = n - 1 < s.wait_cap ? n - 1 : s.wait_cap;
    if (n - 1 > s.wait_cap) LOG_WARN("log_rx(%d): buffer too small (%zu < %zu)", port, s.wait_cap, n - 1);
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

// Consume received bytes up to and including one frame delimiter. Returns false when the RX
// ring is empty.
static bool rx_step(void) {
  size_t n = log_port_rx_avail();
  if (n == 0) return false;
  const uint8_t *b = log_port_rx_peek();
  const uint8_t *z = memchr(b, 0, n);
  size_t take = z ? (size_t)(z - b) : n;
  // Bytes before the first 00 belong to a frame whose start was missed.
  if (s.synced) {
    if (s.frame_n + take <= sizeof(s.frame)) {
      memcpy(s.frame + s.frame_n, b, take);
      s.frame_n += take;
    } else {
      s.oversize = true;
      s.frame_n = sizeof(s.frame);
    }
  }
  if (z == NULL) {
    log_port_rx_skip(n);
    return true;
  }
  log_port_rx_skip(take + 1);
  if (s.synced && s.frame_n > 0) {
    ssize_t d = s.oversize ? -1 : cobs_dec(s.frame, s.frame, s.frame_n);
    // A 1450-byte run can decode to up to 1449 bytes: over the payload limit (protocol.md §2.2).
    if (d > LOG_MAX_PACKET_SIZE + 1) s.oversize = true, d = -1;
    if (d < 0) {
      s.rx_errors++;
      LOG_WARN("bad frame from host (%zu bytes%s)", s.frame_n, s.oversize ? ", too long" : "");
    } else {
      dispatch(s.frame, (size_t)d);
    }
  }
  s.synced = true;
  s.oversize = false;
  s.frame_n = 0;
  return true;
}

void log_poll(void) {
  if (s.in_dispatch || !s.started) return; // from a handler: the frame buffer is in use
  if (log_port_rx_check() != 0) {
    s.rx_uart_errors++;
    s.synced = false; // bytes were lost: resynchronise on the next 00
    s.frame_n = 0;
  }
  if (s.tx_dropped != s.tx_dropped_reported && cb_write_avail(&tx_cb) > LOG_MAX_LOG_SIZE * 2) {
    uint32_t d = s.tx_dropped;
    LOG_WARN("TX buffer full: %lu frames dropped", (unsigned long)(d - s.tx_dropped_reported));
    s.tx_dropped_reported = d;
  }
  while (rx_step()) {
    // A handler may have completed a log_rx() wait; let it see its reply first.
    if (s.wait_done) return;
  }
}

void log_notify(uint8_t port, log_cb_t *cb, void *ctx) {
  if (port > 63) LOG_FATAL("port out of range: %d", port);
  s.contexts[port] = ctx;
  s.handlers[port] = cb;
}

size_t log_rx(uint8_t port, uint8_t *data, size_t n) {
  if (port > 63) LOG_FATAL("invalid port %d", port);
  if (s.in_dispatch || s.wait_port >= 0) {
    LOG_ERROR("log_rx(%d) called from a handler or nested", port);
    return 0;
  }
  s.wait_data = data;
  s.wait_cap = n;
  s.wait_done = false;
  s.wait_port = port;
  size_t r = 0;
  while (true) {
    log_poll();
    if (s.wait_done) {
      r = s.wait_n;
      break;
    }
    if (!s.host) {
      LOG_WARN("log_rx(%d): host went away", port);
      break;
    }
  }
  s.wait_port = -1;
  return r;
}
