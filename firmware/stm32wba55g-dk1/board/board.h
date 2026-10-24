#pragma once

#include <stdbool.h>
#include <stdint.h>

#define BOARD_NAME "stm32wba55g_dk1"
#define BLE_NAME "ucLog test STM32WBA55G-DK1" // advertised (scan response) and the GAP device name

void board_init(void);
void board_led(bool on);
void board_led_toggle(void);
uint32_t board_ms(void);
void board_delay_ms(uint32_t d);
const uint8_t *board_uid(void); // 12 bytes
const char *board_link(void);   // the link in use: "uart" or "ble"

bool board_lock(void);            // mask all but the radio interrupts; true if none were masked
void board_unlock(bool enabled); // restore the state board_lock() returned (the link layer
                                 // library has its own disable_irq/enable_irq)
