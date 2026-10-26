// © 2025 Unit Circle Inc.`

#include <stdlib.h>
#include <stdint.h>
#include "zephyr/kernel.h"
#include "zephyr/logging/log.h"
#include "zephyr/logging/log_output.h"
#include "zephyr/timing/timing.h"

#include <stdarg.h>

LOG_MODULE_REGISTER(main);

void my_work_handler(struct k_work *work) {
  //int64_t now = k_uptime_get();
  //double  secs = (float) now / 1000.0f;
  static uint32_t x  = 0;
  x = x + 1;
  timing_start();
  timing_t start = timing_counter_get();
  //LOG_INF("tick %.3f s since reset", secs);
  //LOG_INF("tick %lld s since reset", now);
  LOG_INF("tick 00000 s since reset");
  //LOG_INF("tick %u since reset", x);
  timing_t end = timing_counter_get();
  timing_stop();
  uint64_t cycles = timing_cycles_get(&start, &end);
  uint64_t ns     = timing_cycles_to_ns(cycles);
  LOG_INF("timing %lld cycles %lld ns", cycles, ns);
}

K_WORK_DEFINE(my_work, my_work_handler);

void my_timer_handler(struct k_timer *dummy) {
    k_work_submit(&my_work);
}

K_TIMER_DEFINE(my_timer, my_timer_handler, NULL);

int main(void) {
    //log_output_timestamp_freq_set(0); // Disable generating timestamps
  LOG_ERR("error");
  LOG_WRN("warn");
  LOG_INF("info");
  LOG_DBG("debug");
  //printk("xxx %s %d\n", "hello", 2);
  //LOG_PRINTK("hello\n"); // calls Z_LOG_PRINTK - can easily fake it
  timing_init();
  (void) k_timer_start(&my_timer, K_SECONDS(5), K_SECONDS(5));

  while (true) {
    (void) k_sleep(K_MSEC(1000));
    //boot_banner();
    LOG_DBG("debug");
    LOG_DEBUG("%s", "hello there");

  }
}




//----------------

#if 0

static void uc_console_hook_install(void)
{
#if defined(CONFIG_STDOUT_CONSOLE)
        __stdout_hook_install(console_out);
#endif
#if defined(CONFIG_PRINTK)
        __printk_hook_install(console_out);
#endif
}

/**
 * @brief Initialize one UART as the console/debug port
 *
 * @return 0 if successful, otherwise failed.
 */
static int uc_console_init(void)
{
        if (!device_is_ready(uart_console_dev)) {
                return -ENODEV;
        }

        uart_console_hook_install();

        return 0;
}

/* UART console initializes after the UART device itself */
SYS_INIT(uc_console_init,
#if defined(CONFIG_EARLY_CONSOLE)
         PRE_KERNEL_1,
#else
         POST_KERNEL,
#endif
         CONFIG_CONSOLE_INIT_PRIORITY);

#endif
