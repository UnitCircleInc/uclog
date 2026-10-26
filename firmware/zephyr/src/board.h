// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// The test app's board interface (../app) on Zephyr.

#pragma once

#include <stdbool.h>
#include <stdint.h>

#define BOARD_NAME CONFIG_BOARD

void board_init(void);
void board_led(bool on);
void board_led_toggle(void);
uint32_t board_ms(void);
void board_delay_ms(uint32_t d);
const uint8_t *board_uid(void); // 12 bytes
