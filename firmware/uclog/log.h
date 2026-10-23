// © 2025 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// ucLog device API: the essence of include/log.h from github.com/UnitCircleInc/uclog, without
// Zephyr. Wire format: spec/protocol.md.
//
// A log call sends only the address of its format string plus the binary arguments. The
// strings live in the non-loaded `.logdata` section (log.ld); the host decodes them with the
// `.logdata` generated from the ELF (tools/logdata.py).
//
// NOTE: __FILE__ must not contain ':' (the host splits "level:file:line:fmt" on it).
//
// Format extensions:
//   {enum:<name>}%d  - print the enumerator of enum (or typedef'd enum) <name>
//   {sym}%p          - print the function or variable the address points into

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "cb.h"

#ifdef __cplusplus
extern "C" {
#endif

// ---------------------------------------------------------------------------------------------
// Configuration (override with -D)

#if !defined(LOG_TX_BUF_SIZE)
#define LOG_TX_BUF_SIZE (8192) // log ring buffer (frames wait here until the host takes them)
#endif

// Largest port data (log_tx/log_rx), so that every frame fits in 1452 bytes on the wire: one
// unfragmented UDP datagram on a 1500-byte MTU with IPv6 (spec/protocol.md §2.4). The Zephyr
// version used 1500.
#define LOG_MAX_PACKET_SIZE (1443)
#define LOG_MAX_FRAME_SIZE (1452) // 00 COBS(port byte + 1443 bytes) 00

#if !defined(LOG_MAX_LOG_SIZE)
#define LOG_MAX_LOG_SIZE (100) // largest log frame payload; longer strings are truncated
#endif

// ---------------------------------------------------------------------------------------------
// Logging

#define LOG_LVL_DEBUG 0
#define LOG_LVL_INFO 1
#define LOG_LVL_WARN 2
#define LOG_LVL_ERROR 3
#define LOG_LVL_FATAL 4
#define LOG_LVL_PANIC 5

#define LOG_DEBUG(...) LOG_(LOG_LVL_DEBUG, VA_SEL(__VA_ARGS__), __VA_ARGS__)
#define LOG_INFO(...) LOG_(LOG_LVL_INFO, VA_SEL(__VA_ARGS__), __VA_ARGS__)
#define LOG_WARN(...) LOG_(LOG_LVL_WARN, VA_SEL(__VA_ARGS__), __VA_ARGS__)
#define LOG_ERROR(...) LOG_(LOG_LVL_ERROR, VA_SEL(__VA_ARGS__), __VA_ARGS__)
#define LOG_FATAL(...)                                                                         \
  do {                                                                                         \
    log_panic_();                                                                              \
    LOG_(LOG_LVL_FATAL, VA_SEL(__VA_ARGS__), __VA_ARGS__);                                     \
    log_fatal_();                                                                              \
  } while (false)

// Memory dumps: "<fmt> <addr>: xx xx ..." (at most LOG_MAX_LOG_SIZE - 8 bytes).
#define LOG_MEM_DEBUG(fmt_, buf_, n_) LOG_MEM_(LOG_LVL_DEBUG, fmt_, buf_, n_)
#define LOG_MEM_INFO(fmt_, buf_, n_) LOG_MEM_(LOG_LVL_INFO, fmt_, buf_, n_)
#define LOG_MEM_WARN(fmt_, buf_, n_) LOG_MEM_(LOG_LVL_WARN, fmt_, buf_, n_)
#define LOG_MEM_ERROR(fmt_, buf_, n_) LOG_MEM_(LOG_LVL_ERROR, fmt_, buf_, n_)
#define LOG_MEM_FATAL(fmt_, buf_, n_)                                                          \
  do {                                                                                         \
    log_panic_();                                                                              \
    LOG_MEM_(LOG_LVL_FATAL, fmt_, buf_, n_);                                                   \
    log_fatal_();                                                                              \
  } while (false)

// ---------------------------------------------------------------------------------------------
// Link

// Call first: logs are buffered from here on.
void log_pre_init(void);
// Start the link (log_port_init). `board` and `image_type` go into the port-62 device info.
void log_init(const char *board, const char *image_type);

// Process received frames: dispatches to log_notify() handlers. Call from the main loop.
void log_poll(void);

// Call every millisecond (e.g. from SysTick): idle pulses and host liveness.
void log_tick(uint32_t now_ms);

// Send `data` (at most LOG_MAX_PACKET_SIZE bytes) on `port` (0..63). The frame is dropped (and
// counted) when the buffer is full; check log_tx_avail() first if it must not be.
void log_tx(uint8_t port, const uint8_t *data, size_t n);
size_t log_tx_avail(void);
// Everything queued has been handed to the link: the buffer is empty and no TX is in progress.
// Stays false while no host is connected (TX is paused).
bool log_tx_flushed(void);

// Wait for the next frame on `port`. Copies at most `n` bytes and returns the payload length.
// Returns 0 if the host goes away while waiting, or when called from a handler.
size_t log_rx(uint8_t port, uint8_t *data, size_t n);

// Handler for frames received on `port`. Runs in log_poll(); `rx` is valid only during the call.
typedef void log_cb_t(const uint8_t *rx, size_t rx_n, void *ctx);
void log_notify(uint8_t port, log_cb_t *cb, void *ctx);

// The host is present (bytes received in the last 2 s). Logs are buffered while it is not.
int log_is_host_ready(bool *host_ready);

// Queue the port-62 device info again.
void log_send_device_info(void);

#define LOG_APP_HASH_SIZE 64
const uint8_t *log_app_hash(size_t *n);

// Counters for diagnostics.
typedef struct {
  uint32_t tx_dropped;    // frames dropped because the TX buffer was full
  uint32_t tx_bytes;      // bytes queued for the link (frames and pulses)
  uint32_t rx_uart_errors; // UART framing/noise errors (RX restarted)
  uint32_t rx_frames;     // frames received (excluding pulses)
  uint32_t rx_errors;     // COBS / oversize errors
  uint32_t rx_no_handler;
} log_stats_t;
void log_get_stats(log_stats_t *s);

// ---------------------------------------------------------------------------------------------
// Port (board) interface: implemented by the board, called by log.c. Mirrors the Zephyr ucuart
// API: TX drains `tx_cb` (e.g. by DMA), RX is a ring buffer read with avail/peek/skip.

uint32_t log_port_lock(void);                          // disable interrupts, return previous state
void log_port_unlock(uint32_t key);                    // restore
uint32_t log_port_ms(void);                            // millisecond counter
void log_port_init(cb_t *tx_cb);                       // start the UART; TX takes data from tx_cb
void log_port_tx_schedule(const uint8_t *prefix, size_t n); // start TX: prefix (if any), then tx_cb
void log_port_tx_pause(bool pause);                    // stop starting new TX transfers
bool log_port_tx_idle(void);                           // no TX transfer in progress
void log_port_panic(void);                             // switch to polled TX (interrupts off)
size_t log_port_rx_avail(void);                        // contiguous received bytes
const uint8_t *log_port_rx_peek(void);
void log_port_rx_skip(size_t n);
uint32_t log_port_rx_mark(void);                       // changes whenever a byte is received
uint32_t log_port_rx_check(void);                      // restart RX after UART errors; nonzero if any
__attribute__((noreturn)) void log_port_fatal(void);   // breakpoint if debugging, else reset

// Optional override: runs before the reset in LOG_FATAL.
void log_fatal(void);

// ---------------------------------------------------------------------------------------------
// Implementation details

#define VA_NARGS_IMPL(_1, _2, _3, _4, _5, _6, _7, _8, _9, _10, _11, _12, _13, _14, _15, N, ...) N
#define VA_SEL(...) VA_NARGS_IMPL(__VA_ARGS__, N, N, N, N, N, N, N, N, N, N, N, N, N, N, 1)

#define TOSTR_(x_) TOSTR1_(x_)
#define TOSTR1_(x_) #x_

#define LOG_STRING_(x_)                                                                        \
  (__extension__({                                                                             \
    static const __attribute__((__aligned__(4), __section__(".logstr." TOSTR_(__LINE__))))   \
    char c__[] = (x_);                                                                         \
    (const char *)&c__;                                                                        \
  }))

#define LOG_(c_, s_, ...) LOG_IMPL_(c_, s_, __VA_ARGS__)
#define LOG_IMPL_(c_, s_, ...) LOG##s_##_(c_, __VA_ARGS__)

#define LOG1_(c_, fmt_)                                                                        \
  do {                                                                                         \
    log_fmt_chk_(fmt_);                                                                        \
    log_log1_(LOG_STRING_(#c_ ":" __FILE__ ":" TOSTR_(__LINE__) ":" fmt_));                   \
  } while (false)

#define LOGN_(c_, fmt_, ...)                                                                   \
  do {                                                                                         \
    const char mfmt_[] = {MAP(typechar, __VA_ARGS__) 0};                                      \
    log_fmt_chk_(fmt_, __VA_ARGS__);                                                           \
    log_logn_(mfmt_, LOG_STRING_(#c_ ":" __FILE__ ":" TOSTR_(__LINE__) ":" fmt_), __VA_ARGS__); \
  } while (false)

#define LOG_MEM_(c_, fmt_, buf_, n_) LOG_MEM_IMPL_(c_, fmt_, buf_, n_) // expands c_ first
#define LOG_MEM_IMPL_(c_, fmt_, buf_, n_)                                                      \
  do {                                                                                         \
    log_mem_(LOG_STRING_(#c_ ":" __FILE__ ":" TOSTR_(__LINE__) ":" fmt_), buf_, n_);          \
  } while (false)

static inline void __attribute__((always_inline, format(printf, 1, 2)))
log_fmt_chk_(__attribute__((unused)) const char *fmt, ...) {}

void log_log1_(const char *prefix);
void log_logn_(const char *kinds, const char *prefix, ...);
void log_mem_(const char *prefix, const void *b, size_t n);
void log_panic_(void);
__attribute__((noreturn)) void log_fatal_(void);

// MAP over up to 20 arguments. Techniques from
//  - https://gustedt.wordpress.com/2010/06/08/detect-empty-macro-arguments/
//  - http://jhnet.co.uk/articles/cpp_magic
#define ARG_CNT_(_0, _1, _2, _3, _4, _5, _6, _7, _8, _9, _10, _11, _12, _13, _14, _15, _16, _17, _18, _19, _20, ...) _20
#define HAS_COMMA(...) ARG_CNT_(__VA_ARGS__, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0)
#define TRIGGER_PARENTHESIS_(...) ,
#define ISEMPTY(...)                                                                           \
  ISEMPTY_(HAS_COMMA(__VA_ARGS__), HAS_COMMA(TRIGGER_PARENTHESIS_ __VA_ARGS__),               \
           HAS_COMMA(__VA_ARGS__(/*empty*/)), HAS_COMMA(TRIGGER_PARENTHESIS_ __VA_ARGS__(/*empty*/)))
#define PASTE5(_0, _1, _2, _3, _4) _0##_1##_2##_3##_4
#define ISEMPTY_(_0, _1, _2, _3) HAS_COMMA(PASTE5(IS_EMPTY_CASE_, _0, _1, _2, _3))
#define IS_EMPTY_CASE_0001 ,

#define UCEMPTY()
#define EVAL(...) EVAL32(__VA_ARGS__)
#define EVAL32(...) EVAL16(EVAL16(__VA_ARGS__))
#define EVAL16(...) EVAL8(EVAL8(__VA_ARGS__))
#define EVAL8(...) EVAL4(EVAL4(__VA_ARGS__))
#define EVAL4(...) EVAL2(EVAL2(__VA_ARGS__))
#define EVAL2(...) EVAL1(EVAL1(__VA_ARGS__))
#define EVAL1(...) __VA_ARGS__
#define DEFER2(m) m UCEMPTY UCEMPTY()()
#define CAT(a, b) a##b
#define IF_NOT(condition) CAT(IF2_, condition)
#define IF2_1(...)
#define IF2_0(...) __VA_ARGS__
#define MAP_(m, first, ...) m(first) IF_NOT(ISEMPTY(__VA_ARGS__))(DEFER2(MAP__)()(m, __VA_ARGS__))
#define MAP__() MAP_
#define MAP(m, ...) EVAL(MAP_(m, __VA_ARGS__))

// Argument kinds, by C type (ARM: int and long are 4 bytes, long double is 8):
// '0' 4-byte int (smaller types are promoted), '1' 8-byte int, '2' double (float is promoted),
// '3' long double, '4' NUL-terminated string, '5' pointer (and anything else).
#define typechar(x)                                                                            \
  _Generic((x),                                                                                \
      _Bool: '0',                                                                              \
      char: '0',                                                                               \
      signed char: '0',                                                                        \
      unsigned char: '0',                                                                      \
      short int: '0',                                                                          \
      unsigned short int: '0',                                                                 \
      int: '0',                                                                                \
      unsigned int: '0',                                                                       \
      long int: '0',                                                                           \
      unsigned long int: '0',                                                                  \
      long long int: '1',                                                                      \
      unsigned long long int: '1',                                                             \
      float: '2',                                                                              \
      double: '2',                                                                             \
      long double: '3',                                                                        \
      char *: '4',                                                                             \
      const char *: '4',                                                                       \
      default: '5'),

#ifdef __cplusplus
}
#endif
