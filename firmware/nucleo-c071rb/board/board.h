#pragma once

#include <stdbool.h>
#include <stdint.h>

#define BOARD_NAME "nucleo_c071rb"

void board_init(void);
void board_led(bool on);
void board_led_toggle(void);
uint32_t board_ms(void);
void board_delay_ms(uint32_t d);
const uint8_t *board_uid(void); // 12 bytes
uint32_t board_cycles(void);    // a CPU cycle counter (wraps), for timing short stretches of code
uint32_t board_hz(void);        // CPU cycles per second
const char *board_link(void);   // the link in use: "uart", "cdc" or "hid"

bool disable_irq(void);        // returns true if interrupts were enabled
void enable_irq(bool enabled); // restores the state disable_irq() returned
