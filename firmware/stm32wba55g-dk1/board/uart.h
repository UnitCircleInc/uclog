// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// USART1 for the ucLog link on the STM32WBA55 (PB12 TX, PA8 RX: the STM32WBA55G-DK1's ST-LINK
// virtual COM port), interrupt driven with the 8-byte FIFOs. RX goes into a ring; TX drains the
// log ring buffer, after an optional prefix. The interface of ../nucleo-c071rb/board/uart.h.

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "cb.h"

typedef enum {
  UART_ERROR_NONE = 0,
  UART_ERROR_RX_FULL = 1 << 1,
  UART_ERROR_OR = 1 << 2,
  UART_ERROR_FRAMING = 1 << 3,
  UART_ERROR_NOISE = 1 << 4,
} uart_error_t;

void uart_init(uint32_t baud, cb_t *rx_cb);
void uart_irq(void);

void uart_tx_attach(cb_t *tx_cb);
void uart_tx_schedule(const uint8_t *prefix, size_t n);
void uart_tx_pause(bool pause);
bool uart_tx_idle(void);
void uart_tx_panic(void); // polled from now on (interrupts off)

size_t uart_rx_avail(void);
const uint8_t *uart_rx_peek(void);
void uart_rx_skip(size_t n);
void uart_rx_discard(void);
uint32_t uart_rx_count(void);     // bytes received so far
uart_error_t uart_rx_check(void); // the errors seen since the last call
