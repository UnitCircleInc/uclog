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
#include "sbl.h"
#include "lib/thread_watchdog.h"

#if CONFIG_UC_LOG_SAVE
#define NOCLEAR __noinit
#else
#define NOCLEAR
#endif

#if defined(CONFIG_CONTROL_SUBSYSTEM)
#include "subsys/control.h"
#endif

#include <zephyr/kernel.h>
#include <zephyr/init.h>

#define LOG_MAX_IN_PORTS (8)

#include <drivers/ucuart.h>

typedef struct {
  const struct device* uart;
  uint8_t buf[COBS_ENC_SIZE(LOG_MAX_PACKET_SIZE)+3];
  cb_t    cb;
  bool    tx_enabled;
  bool    overrun;
#if LOG_MAX_IN_PORTS > 0
  log_cb_t* handlers[LOG_MAX_PORTS];
  void*     contexts[LOG_MAX_PORTS];
  uint8_t port;
  uint8_t  rx_port;
  bool     rx_avail;
  uint8_t* rx_data;
  size_t   rx_n;
  struct k_event rx_event;
#endif
  K_KERNEL_STACK_MEMBER(thread_stack, CONFIG_UC_LOG_STACK_SIZE);
  struct k_thread thread;
} log_data_t;


static log_data_t log_data;

#if !defined(LOG_SIZE)
#define LOG_SIZE (4096*2)
#endif

static NOCLEAR cb_t    tx_cb;
static NOCLEAR uint8_t tx_buf[LOG_SIZE];


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

void log_tx(uint8_t port, const uint8_t* data, size_t n) {
  static uint8_t b[COBS_ENC_SIZE(LOG_MAX_PACKET_SIZE+1)+2];
  if (n > LOG_MAX_PACKET_SIZE) LOG_FATAL("tx message too long %zu", n);
  if (63 < port) LOG_FATAL("invalid port %d", port);
  b[sizeof(b)-(LOG_MAX_PACKET_SIZE+1)] = (port << 2) | 3;
  memmove(b+sizeof(b)-LOG_MAX_PACKET_SIZE, data, n);
  n = cobs_enc(b+1, b+sizeof(b)-(LOG_MAX_PACKET_SIZE+1), n + 1);
  b[0] = '\0';
  b[n+1] = '\0';
  tx_buffer(b, n+2);
  if (log_data.tx_enabled) ucuart_tx_schedule(log_data.uart);
}

size_t log_tx_avail(void) {
  return cb_write_avail(&tx_cb);
}

#if LOG_MAX_IN_PORTS > 0

size_t log_rx(uint8_t port, uint8_t* data, size_t n) {
  if (port >= 64) LOG_FATAL("Invalid port %d", port);
  if (log_data.rx_port != 255) LOG_FATAL("Trying to call log_rx from another thread");
  log_data.rx_data = data;
  log_data.rx_n = n;
  log_data.rx_port = port;

  while (log_data.rx_port != 255) {
    unsigned int key = irq_lock();
    uint32_t r =  k_event_wait(&log_data.rx_event, 1, false, K_MSEC(MAX_WD_WAIT_MS));
    if (r != 0) k_event_clear(&log_data.rx_event, r);
    irq_unlock(key);

    // Feed the watchdog incase we are being called from a thread
    // that has registered with the watchdog service.
    (void) thread_watchdog_feed();
  }

  log_data.rx_port = 255;
  return log_data.rx_n;
}

void log_notify(uint8_t port, log_cb_t* task, void* ctx) {
  if (port >= LOG_MAX_PORTS) {
    LOG_FATAL("port out of range: %d", port);
  }
  log_data.handlers[port] = task;
  log_data.contexts[port] = ctx;
}

static void log_task_entry(log_data_t* data) {
  LOG_INFO("log task");
  cb_init(&data->cb, data->buf, sizeof(data->buf));

  resume_tx();
  //ucuart_on_wakeup_notify(data->uart, task, RX_INT);

run:
  while (true) {
    // Wait for a start of frame
    while (true) {
      size_t n = ucuart_rx_avail(data->uart);

      // It is possible to have a pending UCUART_EVT_RX even though previous handling has
      // drained the queued rx data.
      // E.g.
      // log process wait for UCUART_EVT_RX
      //   post UCUART_EVT_RX avail: 64 (setting event)
      // log process starts running (clearing event)
      // log process wait for UCUART_EVT_RX
      //   post UCUART_EVT_RX avail: 64 (setting event)
      // log process starts running (clearing event)
      //   post UCUART_EVT_RX avail: 3 (setting event)
      // log process continues running - draining queued data
      // log process wait for UCUART_EVT_RX
      // log process starts running (clearing event)
      // log process finds there is no new data.
      while (n == 0u) {
        uint32_t r = ucuart_wait_event(data->uart, UCUART_EVT_RX, false, K_MSEC(MAX_WD_WAIT_MS));
        int32_t wd_rc = thread_watchdog_feed();
        if (wd_rc != 0 && wd_rc != -EBUSY) {
            LOG_ERROR("Couldn't feed watchdog: %d", wd_rc);
        }
        if (data->tx_enabled) ucuart_tx_schedule(data->uart);

        if (r == 0) goto pause;
        n = ucuart_rx_avail(data->uart);
      }

      const uint8_t*b = ucuart_rx_peek(data->uart);
      if (*b != '\0') break;
      ucuart_rx_skip(data->uart, 1);
    }

    // Process until end of frame
    cb_reset(&data->cb);
    data->overrun = false;
    while (true) {
      size_t n = ucuart_rx_avail(data->uart);

      while (n == 0u) {
        uint32_t r = ucuart_wait_event(data->uart, UCUART_EVT_RX, false, K_MSEC(100));
        if (r == 0) goto pause;
        n = ucuart_rx_avail(data->uart);
      }

      const uint8_t* b = ucuart_rx_peek(data->uart);
      const uint8_t* e = memchr(b, '\0', n);
      if (e != NULL) {
        n = e - b;
        if (n > cb_write_avail(&data->cb)) {
          data->overrun = true;
          n = cb_write_avail(&data->cb);
        }
        cb_write(&data->cb, b, n);
        //LOG_MEM_INFO("Rx: ", data->buf,  cb_peek_avail(&data->cb));
        ucuart_rx_skip(data->uart, e-b); // Leave the 0x00 frame terminator
        ssize_t n = cobs_dec(data->buf, data->buf, cb_peek_avail(&data->cb));
        if ((n < 0) || data->overrun) {
          LOG_ERROR("COBS decode error: %d overrun: %d", (int) n, data->overrun);
        }
        else if (n > 0) {
          uint8_t type = data->buf[0] & 3;
          data->port = data->buf[0] >> 2;
          cb_skip(&data->cb, 1);
          if (type != 0x3) {
            LOG_ERROR("unexpected frame type: %d", type);
          }
          else if ((data->rx_port < 64) && (data->port == data->rx_port)) {
            size_t nn = n - 1;
            if (nn > data->rx_n) {
              nn = data->rx_n;
              LOG_WARN("rx_port buffer size too small");
            }
            memmove(data->rx_data, data->buf + 1, nn);
            data->rx_n = n - 1;
            data->rx_port = 255;
            k_event_post(&data->rx_event, 1);
          }
          else if (data->port >= LOG_MAX_PORTS) {
            LOG_ERROR("invalid port: %d", data->port);
          }
          else if (data->handlers[data->port]) {
            data->handlers[data->port](data->buf + 1, n - 1,
                data->contexts[data->port]);
          }
          else {
            LOG_ERROR("no handler for port: %d", data->port);
            LOG_MEM_ERROR("data:", data->buf+1, n-1);
          }
        }
        else {
          LOG_INFO("empty frame");
          // Ignore empty frames
        }
        break;
      }
      else {
        size_t n2 = n;
        if (n > cb_write_avail(&data->cb)) {
          data->overrun = true;
          n = cb_write_avail(&data->cb);
        }
        cb_write(&data->cb, b, n);
        ucuart_rx_skip(data->uart, n2);
      }
    }
  }
pause:
  //LOG_INFO("log pause");
  suspend_tx();
#if 0
  scheduler_clear_events(TX_DONE);
  CO_WAIT_EVENT(task->co_state, TX_DONE | RX_DONE, 0);
  if ((task->active & RX_DONE) == 0) {
    scheduler_set_pollfn(NULL);
    scheduler_set_stopok(true);
    if (ucuart_lp_enter(data->uart)) {
      CO_WAIT_EVENT(task->co_state, RX_INT, 0);
      ucuart_lp_exit(data->uart);
    }
    scheduler_set_stopok(false);
    scheduler_clear_events(RX_DONE);
    scheduler_set_pollfn(log_poll);
  }
#endif
  resume_tx();
  //LOG_INFO("log resume");
  goto run;
}

#endif


static void tx_buffer(const uint8_t* b, size_t n) {
  uint32_t key = irq_lock();
  cb_write(&tx_cb, b, n);
  irq_unlock(key);
}

#if CONFIG_UC_LOG_SAVE_ENABLED
#define APP_HASH_SIZE 64

#ifndef CONFIG_SIGNED_IMAGE
// Will be initialized by apphash.cmake
static const __attribute__((section(".apphash"))) uint8_t app_hash__[APP_HASH_SIZE];
#endif

static NOCLEAR uint8_t app_hash[APP_HASH_SIZE];

static uint8_t saved_app_hash[APP_HASH_SIZE];
static uint8_t saved_log[LOG_SIZE];
static size_t saved_log_n;

const uint8_t* log_saved_log(size_t* n) {
  *n = saved_log_n;
  return saved_log;
}

const uint8_t* log_saved_app_hash(size_t* n) {
  *n = APP_HASH_SIZE;
  return saved_app_hash;
}

void log_tx_saved_log(void) {
  tx_buffer(saved_log, saved_log_n);
}

// app_hash__ and app_hash will only be different on a code change.
// We don't want previous log details for code changes.
static bool log_valid(void) {
#ifdef CONFIG_SIGNED_IMAGE
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

static int log_pre_init(void) {
#if CONFIG_UC_LOG_SAVE_ENABLED
  saved_log_n = 0;
  if (log_valid()) log_save();

#ifdef CONFIG_SIGNED_IMAGE
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
#if LOG_MAX_IN_PORTS > 0
  memset(log_data.handlers, 0, sizeof(log_data.handlers));
#endif
  LOG_INFO("log-pre-init");
  return 0;
}

SYS_INIT(log_pre_init, EARLY, CONFIG_KERNEL_INIT_PRIORITY_DEFAULT);

static const struct device* console = DEVICE_DT_GET_OR_NULL(DT_CHOSEN(zephyr_console));

static int log_post_init(void) {

  if (!device_is_ready(console)) return -ENOTSUP;

  //uart->tx_cb = &tx_cb;
  log_data.rx_port = 255;
  k_event_init(&log_data.rx_event);
  log_data.uart = console;
  ucuart_set_tx_cb(log_data.uart, &tx_cb);

#if LOG_MAX_IN_PORTS == 0
  resume_tx();
#else
  k_tid_t tid = k_thread_create(&log_data.thread, log_data.thread_stack,
                        CONFIG_UC_LOG_STACK_SIZE,
                        (k_thread_entry_t)log_task_entry,
                        &log_data, NULL, NULL,
                        K_PRIO_COOP(CONFIG_UC_LOG_THREAD_PRIORITY),
                        0, K_NO_WAIT);
  if (k_thread_name_set(tid, "Log") != 0) {
      // Couldn't set thread name
  }

  int32_t rc = thread_watchdog_install(tid);
  if (rc != 0) {
      LOG_FATAL("Couldn't install watchdog: %d", rc);
  }
#endif
  return 0;
}
SYS_INIT(log_post_init, APPLICATION, 10);
