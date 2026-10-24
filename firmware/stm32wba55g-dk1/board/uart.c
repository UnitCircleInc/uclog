// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// USART1 (PB12 TX, PA8 RX: the ST-LINK virtual COM port), interrupt driven with the FIFOs, on the
// STM32WBA55 (uart.h).

#include "uart.h"

#include <stm32wbaxx.h>

#include "board.h"
#include "log.h"

#define UART USART1

static struct {
  cb_t *tx_cb, *rx_cb;
  const uint8_t *prefix;
  size_t prefix_n;
  bool busy, paused, panic;
  volatile uart_error_t error;
  volatile uint32_t rx_total;
} u;

void uart_init(uint32_t baud, cb_t *rx_cb) {
  u.rx_cb = rx_cb;
  RCC->AHB2ENR |= RCC_AHB2ENR_GPIOAEN | RCC_AHB2ENR_GPIOBEN;
  RCC->APB2ENR |= RCC_APB2ENR_USART1EN;
  RCC->APB2RSTR |= RCC_APB2RSTR_USART1RST;
  RCC->APB2RSTR &= ~RCC_APB2RSTR_USART1RST;

  // PB12 TX, PA8 RX: alternate function 7, pull-up, very high speed.
  GPIOB->MODER = (GPIOB->MODER & ~GPIO_MODER_MODE12) | GPIO_MODER_MODE12_1;
  GPIOB->AFR[1] = (GPIOB->AFR[1] & ~GPIO_AFRH_AFSEL12) | (7u << GPIO_AFRH_AFSEL12_Pos);
  GPIOB->PUPDR = (GPIOB->PUPDR & ~GPIO_PUPDR_PUPD12) | GPIO_PUPDR_PUPD12_0;
  GPIOB->OSPEEDR |= GPIO_OSPEEDR_OSPEED12;
  GPIOA->MODER = (GPIOA->MODER & ~GPIO_MODER_MODE8) | GPIO_MODER_MODE8_1;
  GPIOA->AFR[1] = (GPIOA->AFR[1] & ~GPIO_AFRH_AFSEL8) | (7u << GPIO_AFRH_AFSEL8_Pos);
  GPIOA->PUPDR = (GPIOA->PUPDR & ~GPIO_PUPDR_PUPD8) | GPIO_PUPDR_PUPD8_0;
  GPIOA->OSPEEDR |= GPIO_OSPEEDR_OSPEED8;

  UART->BRR = (SystemCoreClock + baud / 2) / baud; // kernel clock PCLK2 = SYSCLK, 16x oversampling
  UART->CR3 = USART_CR3_ONEBIT | USART_CR3_EIE;
  UART->CR1 = USART_CR1_FIFOEN | USART_CR1_RXNEIE_RXFNEIE | USART_CR1_TE | USART_CR1_RE | USART_CR1_UE;
  while (!(UART->ISR & USART_ISR_TEACK) || !(UART->ISR & USART_ISR_REACK)) {
  }
  NVIC_SetPriority(USART1_IRQn, 2);
  NVIC_EnableIRQ(USART1_IRQn);
  LOG_INFO("uart: USART1 at %lu baud", (unsigned long)baud);
}

void uart_tx_attach(cb_t *tx_cb) {
  u.tx_cb = tx_cb;
}

// Fill the TX FIFO: the prefix first, then the ring. Returns false when there is nothing to send.
static bool tx_fill(void) {
  bool any = false;
  while (UART->ISR & USART_ISR_TXE_TXFNF) {
    if (u.prefix_n > 0) {
      UART->TDR = *u.prefix++;
      u.prefix_n--;
    } else if (!u.paused && u.tx_cb && cb_peek_avail(u.tx_cb) > 0) {
      UART->TDR = *(const uint8_t *)cb_peek(u.tx_cb);
      cb_skip(u.tx_cb, 1);
    } else {
      return any;
    }
    any = true;
  }
  return true;
}

void uart_irq(void) {
  uint32_t isr = UART->ISR;
  while (isr & USART_ISR_RXNE_RXFNE) {
    uint8_t c = (uint8_t)UART->RDR;
    if (cb_write_avail(u.rx_cb) > 0) {
      cb_write(u.rx_cb, &c, 1);
      u.rx_total++;
    } else {
      u.error |= UART_ERROR_RX_FULL;
    }
    isr = UART->ISR;
  }
  uint32_t errors = isr & (USART_ISR_ORE | USART_ISR_FE | USART_ISR_NE);
  if (errors) {
    if (errors & USART_ISR_ORE) u.error |= UART_ERROR_OR;
    if (errors & USART_ISR_FE) u.error |= UART_ERROR_FRAMING;
    if (errors & USART_ISR_NE) u.error |= UART_ERROR_NOISE;
    UART->ICR = USART_ICR_ORECF | USART_ICR_FECF | USART_ICR_NECF;
  }
  if (UART->CR1 & USART_CR1_TXEIE_TXFNFIE) {
    if (!tx_fill()) {
      // Nothing more: done when the last byte has left the shift register.
      UART->CR1 = (UART->CR1 & ~USART_CR1_TXEIE_TXFNFIE) | USART_CR1_TCIE;
    }
  } else if ((UART->CR1 & USART_CR1_TCIE) && (isr & USART_ISR_TC)) {
    UART->ICR = USART_ICR_TCCF;
    UART->CR1 &= ~USART_CR1_TCIE;
    u.busy = false;
    uart_tx_schedule(NULL, 0); // more may have been written meanwhile
  }
}

void uart_tx_schedule(const uint8_t *prefix, size_t n) {
  if (prefix != NULL) {
    u.prefix = prefix;
    u.prefix_n = n;
  }
  if (u.panic) {
    // Polled, interrupts off: the prefix, then everything buffered.
    while (u.prefix_n > 0 || (u.tx_cb && cb_peek_avail(u.tx_cb) > 0)) {
      while (!(UART->ISR & USART_ISR_TXE_TXFNF)) {
      }
      if (u.prefix_n > 0) {
        UART->TDR = *u.prefix++;
        u.prefix_n--;
      } else {
        UART->TDR = *(const uint8_t *)cb_peek(u.tx_cb);
        cb_skip(u.tx_cb, 1);
      }
    }
    while (!(UART->ISR & USART_ISR_TC)) {
    }
  } else if (!(UART->CR1 & USART_CR1_TXEIE_TXFNFIE)) {
    if (u.prefix_n > 0 || (!u.paused && u.tx_cb && cb_peek_avail(u.tx_cb) > 0)) {
      u.busy = true;
      UART->CR1 = (UART->CR1 & ~USART_CR1_TCIE) | USART_CR1_TXEIE_TXFNFIE;
    }
  }
}

void uart_tx_pause(bool pause) {
  u.paused = pause;
  if (!pause) uart_tx_schedule(NULL, 0);
}

bool uart_tx_idle(void) {
  return !u.busy;
}

void uart_tx_panic(void) {
  u.panic = true;
  UART->CR1 &= ~(USART_CR1_TXEIE_TXFNFIE | USART_CR1_TCIE);
  u.busy = false;
}

size_t uart_rx_avail(void) {
  return cb_peek_avail(u.rx_cb);
}

const uint8_t *uart_rx_peek(void) {
  return cb_peek(u.rx_cb);
}

void uart_rx_skip(size_t n) {
  cb_skip(u.rx_cb, n);
}

void uart_rx_discard(void) {
  while (uart_rx_avail() > 0) uart_rx_skip(uart_rx_avail());
}

uint32_t uart_rx_count(void) {
  return u.rx_total;
}

uart_error_t uart_rx_check(void) {
  uart_error_t e = u.error;
  if (e != UART_ERROR_NONE) {
    bool en = board_lock();
    u.error = UART_ERROR_NONE;
    board_unlock(en);
  }
  return e;
}
