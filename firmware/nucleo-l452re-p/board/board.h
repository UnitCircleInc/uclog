#pragma once

#include <stdbool.h>
#include <stdint.h>

#define BOARD_NAME "nucleo_l452re_p"

void board_init(void);
void board_led(bool on);
void board_led_toggle(void);
uint32_t board_ms(void);
void board_delay_ms(uint32_t d);
const uint8_t *board_uid(void); // 12 bytes
