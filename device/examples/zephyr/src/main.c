// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// A minimal ucLog app on Zephyr: a log every second, and a terminal (port 61) with two commands.
// ucLog starts itself (no init call); Zephyr's LOG_INF() and friends are ucLog logs
// (CONFIG_LOG_CUSTOM_HEADER), and log.h's LOG_INFO() works too. Connect with any host, e.g.
// `uclog tui`, and type `help` in the Terminal view.

#include <string.h>

#include <zephyr/drivers/gpio.h>
#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>

#include "log.h"

LOG_MODULE_REGISTER(main);

#define TERM_PORT 61 // the terminal (spec/protocol.md §5.7)

typedef enum { LED_OFF, LED_ON, LED_BLINK } led_mode_t;

static const struct gpio_dt_spec led = GPIO_DT_SPEC_GET(DT_ALIAS(led0), gpios);
static led_mode_t led_mode = LED_BLINK;

static void reply(const char *s) {
  log_tx(TERM_PORT, (const uint8_t *)s, strlen(s));
}

// A line from the terminal (the hosts send whole lines, ending in \r). Handlers run in ucLog's
// thread.
static void on_terminal(const uint8_t *rx, size_t n, void *ctx) {
  static char line[32];
  static size_t len;
  for (size_t i = 0; i < n; i++) {
    char c = (char)rx[i];
    if (c != '\r' && c != '\n') {
      if (len < sizeof(line) - 1) line[len++] = c;
      continue;
    }
    line[len] = 0;
    len = 0;
    if (line[0] == 0) continue;
    if (strcmp(line, "led on") == 0) led_mode = LED_ON;
    else if (strcmp(line, "led off") == 0) led_mode = LED_OFF;
    else if (strcmp(line, "led blink") == 0) led_mode = LED_BLINK;
    else {
      reply("commands: led on, led off, led blink\r\n");
      continue;
    }
    LOG_INFO("LED mode {enum:led_mode_t}%d", led_mode); // shown as LED_ON, LED_OFF or LED_BLINK
    reply("ok\r\n");
  }
}

int main(void) {
  gpio_pin_configure_dt(&led, GPIO_OUTPUT_INACTIVE);
  log_notify(TERM_PORT, on_terminal, NULL);
  LOG_INF("hello from %s on %s", "ucLog", CONFIG_BOARD);

  for (uint32_t count = 1;; count++) {
    k_sleep(K_SECONDS(1));
    LOG_INF("tick %u, uptime %.1f s", count, k_uptime_get() / 1000.0);
    if (count % 10 == 0) LOG_WRN("%u ticks without a reset", count);
    gpio_pin_set_dt(&led, led_mode == LED_BLINK ? count & 1 : led_mode == LED_ON);
  }
}
