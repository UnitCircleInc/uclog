// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

#include "board.h"

#include <zephyr/drivers/gpio.h>
#include <zephyr/drivers/hwinfo.h>
#include <zephyr/kernel.h>

static const struct gpio_dt_spec led = GPIO_DT_SPEC_GET_OR(DT_ALIAS(led0), gpios, {0});
static bool led_on;

void board_init(void) {
  if (led.port != NULL && gpio_is_ready_dt(&led)) gpio_pin_configure_dt(&led, GPIO_OUTPUT_INACTIVE);
}

void board_led(bool on) {
  led_on = on;
  if (led.port != NULL) gpio_pin_set_dt(&led, on);
}

void board_led_toggle(void) {
  board_led(!led_on);
}

uint32_t board_ms(void) {
  return k_uptime_get_32();
}

void board_delay_ms(uint32_t d) {
  k_msleep(d);
}

const uint8_t *board_uid(void) {
  static uint8_t uid[12];
  (void)hwinfo_get_device_id(uid, sizeof(uid));
  return uid;
}
