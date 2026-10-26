// © 2025 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// The Zephyr port of the ucLog device library: log_port.h over a ucuart/ucuart_z/ucusb device
// (devicetree chosen uclog,link, else zephyr,console).
//
// - log_pre_init() at PRE_KERNEL_1, so drivers can log while they start; uclog_init() and the log
//   thread at APPLICATION.
// - The log thread runs log_poll() and log_tick(): it wakes when the driver posts UCUART_EVT_RX,
//   and at least every TICK_MS. Each RX event moves rx_mark (host presence, log.c).
// - log_rx() from another thread blocks on a semaphore until the log thread dispatches the reply
//   (or the host goes away); callers take turns.
// - printk (and stdout without CONFIG_UC_SYSCALLS) are logged line by line.

#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk-hooks.h>
#include <zephyr/sys/libc-hooks.h>

#include "log.h"
#include "ucuart.h"

#if defined(CONFIG_CPU_CORTEX_M)
#include <cmsis_core.h>
#endif

#if DT_HAS_CHOSEN(uclog_link)
#define LINK_NODE DT_CHOSEN(uclog_link)
#else
#define LINK_NODE DT_CHOSEN(zephyr_console)
#endif

#define TICK_MS 10

static const struct device *const link = DEVICE_DT_GET(LINK_NODE);
static cb_t *tx_cb;
static bool panic_mode;
static uint32_t rx_mark;

static K_KERNEL_STACK_DEFINE(log_stack, CONFIG_UC_LOG_STACK_SIZE);
static struct k_thread log_thread;
static k_tid_t log_tid;
static K_MUTEX_DEFINE(rx_mutex);
static K_SEM_DEFINE(rx_sem, 0, 1);

// ---------------------------------------------------------------------------------------------
// log_port.h

uint32_t log_port_lock(void) {
  return irq_lock();
}

void log_port_unlock(uint32_t key) {
  irq_unlock(key);
}

uint32_t log_port_ms(void) {
  return k_uptime_get_32();
}

void log_port_init(cb_t *cb) {
  tx_cb = cb;
  ucuart_set_tx_cb(link, cb);
}

void log_port_tx_schedule(const uint8_t *prefix, size_t n) {
  if (panic_mode) {
    ucuart_panic(link); // sends tx_cb synchronously
  } else {
    ucuart_tx_schedule(link, prefix, n);
  }
}

void log_port_tx_pause(bool pause) {
  ucuart_tx_pause(link, pause);
}

bool log_port_tx_idle(void) {
  return ucuart_tx_idle(link, tx_cb);
}

void log_port_panic(void) {
  panic_mode = true;
}

size_t log_port_rx_avail(void) {
  return ucuart_rx_avail(link);
}

const uint8_t *log_port_rx_peek(void) {
  return ucuart_rx_peek(link);
}

void log_port_rx_skip(size_t n) {
  ucuart_rx_skip(link, n);
}

uint32_t log_port_rx_mark(void) {
  return rx_mark;
}

uint32_t log_port_rx_check(void) {
  return 0; // the drivers restart RX themselves
}

__attribute__((noreturn)) void log_port_fatal(void) {
#if defined(CONFIG_CPU_CORTEX_M)
  // Everything was flushed in panic mode. Breakpoint if a debugger is attached, else reset.
  if ((CoreDebug->DHCSR & CoreDebug_DHCSR_C_DEBUGEN_Msk) != 0) {
    __BKPT(0);
  }
  NVIC_SystemReset();
#else
  k_fatal_halt(K_ERR_KERNEL_PANIC);
#endif
}

bool log_port_is_poller(void) {
  return log_tid == NULL || k_current_get() == log_tid;
}

void log_port_yield(void) {
  k_msleep(1);
}

void log_port_rx_begin(void) {
  k_mutex_lock(&rx_mutex, K_FOREVER);
  k_sem_reset(&rx_sem);
}

void log_port_rx_end(void) {
  k_mutex_unlock(&rx_mutex);
}

void log_port_rx_wait(void) {
  if (log_port_is_poller()) {
    log_poll(); // before the log thread starts
  } else {
    (void)k_sem_take(&rx_sem, K_MSEC(100));
  }
}

void log_port_rx_wake(void) {
  k_sem_give(&rx_sem);
}

// ---------------------------------------------------------------------------------------------
// Watchdog hooks (optional, as before device-v4)

__weak void log_watchdog_feed(void) {}

__weak void log_watchdog_register(const void *thread) {
  ARG_UNUSED(thread);
}

// The image type in the device info; an app can override it.
__weak const char *get_image_type(void) {
  return "app";
}

// ---------------------------------------------------------------------------------------------
// The log thread

static void log_run(void *a, void *b, void *c) {
  ARG_UNUSED(a);
  ARG_UNUSED(b);
  ARG_UNUSED(c);
  while (true) {
    if (ucuart_wait_event(link, UCUART_EVT_RX, false, K_MSEC(TICK_MS)) != 0) rx_mark++;
    log_tick(k_uptime_get_32());
    log_poll();
    log_watchdog_feed();
  }
}

// ---------------------------------------------------------------------------------------------
// printk and stdout as logs

#if defined(CONFIG_PRINTK) || (defined(CONFIG_STDOUT_CONSOLE) && !defined(CONFIG_UC_SYSCALLS))

static char line[LOG_MAX_LOG_SIZE - 4 - 1]; // room for the format address and the NUL
static size_t line_n;

static int console_out(int c) {
  uint32_t key = irq_lock();
  if (c != '\n' && c != '\r') line[line_n++] = (char)c;
  if (c == '\n' || line_n == sizeof(line) - 1) {
    line[line_n] = '\0';
    line_n = 0;
    LOG_INFO("%s", line);
  }
  irq_unlock(key);
  return c;
}

#endif

// ---------------------------------------------------------------------------------------------
// Start-up

static int uclog_pre_init(void) {
  log_pre_init();
#if defined(CONFIG_STDOUT_CONSOLE) && !defined(CONFIG_UC_SYSCALLS)
  __stdout_hook_install(console_out);
#endif
#if defined(CONFIG_PRINTK)
  __printk_hook_install(console_out);
#endif
  return 0;
}

SYS_INIT(uclog_pre_init, PRE_KERNEL_1, CONFIG_KERNEL_INIT_PRIORITY_DEFAULT);

static int uclog_start(void) {
  if (!device_is_ready(link)) return -ENODEV;
  uclog_init(CONFIG_BOARD, get_image_type());
  log_tid = k_thread_create(&log_thread, log_stack, K_KERNEL_STACK_SIZEOF(log_stack), log_run,
                            NULL, NULL, NULL, K_PRIO_COOP(CONFIG_UC_LOG_THREAD_PRIORITY), 0,
                            K_NO_WAIT);
  k_thread_name_set(log_tid, "uclog");
  log_watchdog_register(log_tid);
  return 0;
}

SYS_INIT(uclog_start, APPLICATION, 0);
