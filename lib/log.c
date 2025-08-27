// © 2025 Unit Circle Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#include <stddef.h>
#include <string.h>
#include <stdarg.h>
#include <stdint.h>

#include "log.h"
#include "cobs.h"
#include "cb.h"

#if CONFIG_UC_LOG_SAVE
#define NOCLEAR __noinit
#else
#define NOCLEAR
#endif

#if !defined(CONFIG_UC_LOG_MAX_PACKET_SIZE)
#define CONFIG_UC_LOG_MAX_PACKET_SIZE (1500)
#endif

typedef struct {
  const uart_t* uart;
  bool    tx_enabled;
} log_data_t;

static log_data_t log_data;

#if !defined(CONFIG_UC_LOG_BUF_SIZE)
#define CONFIG_UC_LOG_BUF_SIZE (4096*2)
#endif

static NOCLEAR cb_t    tx_cb;
static NOCLEAR uint8_t tx_buf[CONFIG_UC_LOG_BUF_SIZE];

static size_t strnlen_s (const char* s, size_t n) {
  const char* found = memchr(s, '\0', n);
  return found ? (size_t)(found-s) : n;
}

void log_panic_(void) {
  if (log_data.uart != NULL) {
    ucuart_panic(log_data.uart);
  }
}

static void tx_buffer(const uint8_t* b, size_t n);

void log_log1_(const char *prefix) {
  union {
    const void* p;
    uint8_t v[sizeof(const void*)];
  } v;
  uint8_t b[5+2];

  v.p = prefix;
  v.v[0] = (v.v[0] & 0xfc) | 0x00;
  memmove(b+2, v.v, 4);
  size_t n = cobs_enc(b+1, b+2, 4); // inplace
  b[0] = 0x00;
  b[1+n] = 0x00;
  tx_buffer(b, n+2);
  if (log_data.tx_enabled) ucuart_tx_schedule(log_data.uart);
}

void log_logn_(const char* fmt, const char *prefix,  ...) {
  union {
    unsigned int u;
    unsigned long long int ull;
    double d;
    long double ld;
    const void* p;
    uint8_t v[16];
  } v;

  uint8_t b[100]; // Limits total packet size - code below expect less than 253
  size_t n = sizeof(b)-1-2;
  uint8_t* bb = b+1+1;
  size_t sn;

  v.p = prefix;
  v.v[0] = (v.v[0] & 0xfc) | 0x00;
  memmove(bb, v.v, 4);
  bb += 4; n -= 4;

  va_list args;
  va_start(args, prefix);
  while (*fmt != '\0') {
    switch (*fmt++) {
      case '0':
        if (n < 4) goto done;
        v.u = va_arg(args, unsigned int);
        memmove(bb, v.v, 4);
        bb += 4; n -= 4;
        break;
      case '1':
        if (n < 8) goto done;
        v.ull = va_arg(args, unsigned long long int);
        memmove(bb, v.v, 8);
        bb += 8; n -= 8;
        break;
      case '2':
        if (n < 8) goto done;
        v.d = va_arg(args, double);
        memmove(bb, v.v, 8);
        bb += 8; n -= 8;
        break;
      case '3':
        if (n < 16) goto done;
        v.ld = va_arg(args, long double);
        memmove(bb, v.v, 16);
        bb += 16; n -= 16;
        break;
      case '4':
        if (n < 1) goto done;
        v.p = va_arg(args, char*);
        sn = strnlen_s(v.p, n-1);
        memmove(bb, v.p, sn);
        bb += sn;
        *bb++ = '\0';
        n -= sn + 1;
        break;
      case '5':
        if (n < 4) goto done;
        v.p = va_arg(args, void*);
        memmove(bb, v.v, 4);
        bb += 4; n -= 4;
        break;
      default:
        goto done;
    }
  }
done:
  n = cobs_enc(b+1, b+1+1, sizeof(b) -2 - 1 - n); // inplace
  b[0] = 0x00;
  b[1+n] = 0x00;
  tx_buffer(b, n+2);
  if (log_data.tx_enabled) ucuart_tx_schedule(log_data.uart);
}

void log_mem_(const char *prefix, const void* b, size_t n) {
  union {
    const void* p;
    uint8_t v[sizeof(const void*)];
  } v;
  uint8_t bb[100];

  if (n + 8 + 1 + 2> 100) n = 100 - 8 - 1 - 2;

  v.p = prefix;
  v.v[0] = (v.v[0] & 0xfc) | 0x01;
  memmove(bb+2, v.v, 4);
  v.p = b;
  memmove(bb+2+4, v.v, 4);
  memmove(bb+2+8, b, n);
  n = cobs_enc(bb+1, bb+2, 8+n); // inplace
  bb[0] = 0x00;
  bb[1+n] = 0x00;
  tx_buffer(bb, n+2);
  if (log_data.tx_enabled) ucuart_tx_schedule(log_data.uart);
}

void suspend_tx(void) {
  log_data.tx_enabled = false;
}

void resume_tx(void) {
  log_data.tx_enabled = true;
  ucuart_tx_schedule(log_data.uart);
}

__weak void application_handle_fatal_error(void) {
}

__attribute__((noreturn)) void log_fatal_(void) {
  // Handle it in the control subsystem if implemented
  application_handle_fatal_error();

  // We switched to log_panic so all data will have been flushed
  // If debugger connected then bkpt, otherwise reset
  if ((CoreDebug->DHCSR & CoreDebug_DHCSR_C_DEBUGEN_Msk) != 0) {
    __BKPT(0);
  }
  NVIC_SystemReset();
}

static void tx_buffer(const uint8_t* b, size_t n) {
  uint32_t key = irq_lock();
  cb_write(&tx_cb, b, n);
  irq_unlock(key);
}

#if CONFIG_UC_LOG_SAVE
#define APP_HASH_SIZE 64

#ifndef CONFIG_UC_SIGNED_IMAGE
// Will be initialized by apphash.cmake
static const __attribute__((section(".apphash"))) uint8_t app_hash__[APP_HASH_SIZE];
#endif

static NOCLEAR uint8_t app_hash[APP_HASH_SIZE];

static uint8_t saved_app_hash[APP_HASH_SIZE];
static uint8_t saved_log[CONFIG_UC_LOG_BUF_SIZE];
static size_t saved_log_n;

const uint8_t* log_saved_log(size_t* n) {
  *n = saved_log_n;
  return saved_log;
}

const uint8_t* log_saved_app_hash(size_t* n) {
  *n = APP_HASH_SIZE;
  return saved_app_hash;
}

// app_hash__ and app_hash will only be different on a code change.
// We don't want previous log details for code changes.
static bool log_valid(void) {
#ifdef CONFIG_UC_SIGNED_IMAGE
  const uint8_t *app_hash__ = sbl_app_hash();
#endif

  return (tx_cb.write < tx_cb.n)     && (tx_cb.read < tx_cb.n) &&
         (tx_cb.n == sizeof(tx_buf)) && (tx_cb.b == tx_buf) &&
         (memcmp(app_hash__, app_hash, sizeof(app_hash)) == 0);
}

static void log_save(void) {
  saved_log_n = cb_read_avail(&tx_cb);

  // If it is empty then "force" dumping the entire contents
  if (saved_log_n == 0) {
    cb_skip(&tx_cb, 1);
    saved_log_n = cb_read_avail(&tx_cb);
  }
  if (saved_log_n > sizeof(saved_log)) saved_log_n = sizeof(saved_log);

  uint8_t* save = saved_log;
  size_t log_rem = saved_log_n;
  size_t n = cb_peek_avail(&tx_cb);
  if (n > log_rem) n = log_rem;
  memmove(save, cb_peek(&tx_cb), n);
  save += n;
  cb_skip(&tx_cb, n);
  log_rem -= n;
  n = cb_peek_avail(&tx_cb);
  if (n > log_rem) n = log_rem;
  memmove(save, cb_peek(&tx_cb), n);
  cb_skip(&tx_cb, n);

  // Save the app_hash associated with the log.
  memmove(saved_app_hash, app_hash, sizeof(app_hash));
}
#endif

void log_pre_init(void) {
#if CONFIG_UC_LOG_SAVE
  saved_log_n = 0;
  if (log_valid()) log_save();

#ifdef CONFIG_UC_SIGNED_IMAGE
  const uint8_t *app_hash__ = sbl_app_hash();
#endif

  // Save current app hash so we can get access on next reset
  // app_hash__ might change between resets.
  memmove(app_hash, app_hash__, sizeof(app_hash));
#endif

  log_data.uart = NULL;
  memset(tx_buf, 0, sizeof(tx_buf));
  cb_init(&tx_cb, tx_buf, sizeof(tx_buf));
  suspend_tx();
  LOG_INFO("log-pre-init");
}

void log_init(uart_t* uart) {
  if (uart == NULL) return;

  log_data.uart = uart;
  ucuart_set_tx_cb(log_data.uart, &tx_cb);
  resume_tx();
  // start server if availble
}

#if defined(CONFIG_ZEPHYR_NEWLOG_MODULE)

int zephyr_log_pre_init(void) {
  log_pre_init();
  return 0;
}

SYS_INIT(zephyr_log_pre_init, EARLY, CONFIG_KERNEL_INIT_PRIORITY_DEFAULT);

static const struct device* console = DEVICE_DT_GET_OR_NULL(DT_CHOSEN(zephyr_console));

int zephyr_log_init(void) {
  if (!device_is_ready(console)) return -ENOTSUP;
  log_init(console);
  return 0;
}

SYS_INIT(zephyr_log_init, APPLICATION, 10);
#endif
