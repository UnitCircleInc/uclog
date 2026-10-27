// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// The board (port.c): clock, SysTick, LED. The ucLog port itself is include/log_port.h.

#pragma once

#include <stdbool.h>
#include <stdint.h>

void board_init(void);
void board_led(bool on);
uint32_t board_ms(void);
