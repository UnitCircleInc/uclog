// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// USART2 with DMA for the ucLog link on the STM32C071: the behaviour of the Unit Circle STM32L4
// driver (../nucleo-l452re-p/board/uart.c) used by the L452 board, without low-power, RS-485 and
// flow control. RX: circular DMA (channel 1) into a ring; TX: linear DMA (channel 2) straight from
// the log ring buffer, after an optional prefix.

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "cb.h"

typedef enum {
  UART_ERROR_NONE = 0,
  UART_ERROR_TX_DMA = 1 << 0,
  UART_ERROR_RX_DMA = 1 << 1,
  UART_ERROR_OR = 1 << 2,
  UART_ERROR_FRAMING = 1 << 3,
  UART_ERROR_NOISE = 1 << 4,
} uart_error_t;

void uart_init(uint32_t baud, cb_t *rx_cb);
void uart_irq(void);
void uart_dma_irq(void);

void uart_tx_attach(cb_t *tx_cb);
void uart_tx_schedule(const uint8_t *prefix, size_t n);
void uart_tx_pause(bool pause);
bool uart_tx_idle(void);
void uart_tx_panic(void); // polled from now on (interrupts off)

size_t uart_rx_avail(void);
const uint8_t *uart_rx_peek(void);
void uart_rx_skip(size_t n);
void uart_rx_discard(void);
uint32_t uart_rx_count(void); // bytes received so far (call at least every few ms)
uart_error_t uart_rx_check(void); // restart RX after errors; the errors seen
