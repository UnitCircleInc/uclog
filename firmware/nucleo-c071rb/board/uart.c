// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// USART2 (PA2 TX, PA3 RX: the ST-LINK virtual COM port) with DMA, on the STM32C071 (uart.h).

#include "uart.h"

#include <stm32c0xx.h>

#include "board.h"
#include "log.h"

#define RX_CH DMA1_Channel1
#define TX_CH DMA1_Channel2
#define REQ_USART2_RX 52 // DMAMUX request IDs
#define REQ_USART2_TX 53

static struct {
  cb_t *tx_cb, *rx_cb;
  size_t tx_n;
  const uint8_t *prefix;
  size_t prefix_n;
  bool prefix_busy, paused, panic;
  uart_error_t error;
  uint32_t rx_total;
  size_t rx_pos;
} u;

static void rx_start(void) {
  cb_reset(u.rx_cb);
  u.rx_pos = 0;
  RX_CH->CCR = 0;
  RX_CH->CPAR = (uint32_t)&USART2->RDR;
  RX_CH->CMAR = (uint32_t)u.rx_cb->b;
  RX_CH->CNDTR = u.rx_cb->n;
  RX_CH->CCR = DMA_CCR_MINC | DMA_CCR_CIRC | DMA_CCR_TEIE | DMA_CCR_PL_1 | DMA_CCR_EN;
  USART2->CR3 |= USART_CR3_DMAR | USART_CR3_EIE;
}

void uart_init(uint32_t baud, cb_t *rx_cb) {
  u.rx_cb = rx_cb;
  RCC->IOPENR |= RCC_IOPENR_GPIOAEN;
  RCC->AHBENR |= RCC_AHBENR_DMA1EN;
  RCC->APBENR1 |= RCC_APBENR1_USART2EN;
  RCC->APBRSTR1 |= RCC_APBRSTR1_USART2RST;
  RCC->APBRSTR1 &= ~RCC_APBRSTR1_USART2RST;

  // PA2, PA3: alternate function 1, pull-up, very high speed.
  GPIOA->MODER = (GPIOA->MODER & ~(GPIO_MODER_MODE2 | GPIO_MODER_MODE3)) | GPIO_MODER_MODE2_1 | GPIO_MODER_MODE3_1;
  GPIOA->AFR[0] = (GPIOA->AFR[0] & ~(GPIO_AFRL_AFSEL2 | GPIO_AFRL_AFSEL3)) | (1u << GPIO_AFRL_AFSEL2_Pos) |
                  (1u << GPIO_AFRL_AFSEL3_Pos);
  GPIOA->PUPDR = (GPIOA->PUPDR & ~(GPIO_PUPDR_PUPD2 | GPIO_PUPDR_PUPD3)) | GPIO_PUPDR_PUPD2_0 | GPIO_PUPDR_PUPD3_0;
  GPIOA->OSPEEDR |= GPIO_OSPEEDR_OSPEED2 | GPIO_OSPEEDR_OSPEED3;

  DMAMUX1_Channel0->CCR = REQ_USART2_RX; // DMA channel 1
  DMAMUX1_Channel1->CCR = REQ_USART2_TX; // DMA channel 2

  USART2->BRR = (SystemCoreClock + baud / 2) / baud; // PCLK = SYSCLK, 16x oversampling
  USART2->CR3 = USART_CR3_ONEBIT;
  USART2->CR1 = USART_CR1_TE | USART_CR1_RE | USART_CR1_UE;
  while (!(USART2->ISR & USART_ISR_TEACK) || !(USART2->ISR & USART_ISR_REACK)) {
  }
  NVIC_SetPriority(USART2_IRQn, 0);
  NVIC_EnableIRQ(USART2_IRQn);
  NVIC_SetPriority(DMA1_Channel1_IRQn, 0);
  NVIC_EnableIRQ(DMA1_Channel1_IRQn);
  NVIC_SetPriority(DMA1_Channel2_3_IRQn, 0);
  NVIC_EnableIRQ(DMA1_Channel2_3_IRQn);
  rx_start();
  LOG_INFO("uart: USART2 at %lu baud", (unsigned long)baud);
}

void uart_tx_attach(cb_t *tx_cb) {
  u.tx_cb = tx_cb;
}

// Start the next TX DMA (prefix first), unless paused. No transfer may be in progress.
static void tx_next(void) {
  if (u.paused || u.tx_cb == NULL) return;
  const uint8_t *b;
  size_t n;
  if (u.prefix_n > 0) {
    u.prefix_busy = true;
    b = u.prefix;
    n = u.prefix_n;
  } else {
    n = cb_peek_avail(u.tx_cb);
    if (n == 0) return;
    b = cb_peek(u.tx_cb);
    u.tx_n = n;
  }
  TX_CH->CCR = 0;
  TX_CH->CPAR = (uint32_t)&USART2->TDR;
  TX_CH->CMAR = (uint32_t)b;
  TX_CH->CNDTR = n;
  TX_CH->CCR = DMA_CCR_MINC | DMA_CCR_DIR | DMA_CCR_TCIE | DMA_CCR_TEIE | DMA_CCR_EN;
  USART2->CR3 |= USART_CR3_DMAT;
}

void uart_dma_irq(void) {
  uint32_t isr = DMA1->ISR;
  if (isr & DMA_ISR_TCIF2) {
    DMA1->IFCR = DMA_IFCR_CGIF2;
    TX_CH->CCR = 0;
    USART2->CR3 &= ~USART_CR3_DMAT;
    USART2->CR1 |= USART_CR1_TCIE; // done when the last byte has left the shift register
  } else if (isr & DMA_ISR_TEIF2) {
    DMA1->IFCR = DMA_IFCR_CGIF2;
    TX_CH->CCR = 0;
    u.error |= UART_ERROR_TX_DMA;
  }
  if (isr & DMA_ISR_TEIF1) {
    DMA1->IFCR = DMA_IFCR_CGIF1;
    RX_CH->CCR = 0;
    u.error |= UART_ERROR_RX_DMA;
  }
}

void uart_irq(void) {
  uint32_t isr = USART2->ISR;
  if ((USART2->CR1 & USART_CR1_TCIE) && (isr & USART_ISR_TC)) {
    USART2->ICR = USART_ICR_TCCF;
    USART2->CR1 &= ~USART_CR1_TCIE;
    if (u.prefix_busy) {
      u.prefix_busy = false;
      u.prefix_n = 0;
    } else {
      cb_skip(u.tx_cb, u.tx_n);
    }
    u.tx_n = 0;
    tx_next();
  }
  uint32_t errors = isr & (USART_ISR_ORE | USART_ISR_FE | USART_ISR_NE);
  if (errors && (USART2->CR3 & USART_CR3_EIE)) {
    // Stop RX; log_poll() restarts it through uart_rx_check().
    USART2->CR3 &= ~(USART_CR3_EIE | USART_CR3_DMAR);
    USART2->CR1 &= ~USART_CR1_RE;
    RX_CH->CCR = 0;
    if (errors & USART_ISR_ORE) u.error |= UART_ERROR_OR;
    if (errors & USART_ISR_FE) u.error |= UART_ERROR_FRAMING;
    if (errors & USART_ISR_NE) u.error |= UART_ERROR_NOISE;
    USART2->ICR = USART_ICR_ORECF | USART_ICR_FECF | USART_ICR_NECF;
  }
}

void uart_tx_schedule(const uint8_t *prefix, size_t n) {
  if (prefix != NULL) {
    u.prefix = prefix;
    u.prefix_n = n;
  }
  if (u.panic) {
    // Polled, interrupts off: the prefix, then everything buffered.
    for (size_t i = 0; i < u.prefix_n; i++) {
      while (!(USART2->ISR & USART_ISR_TXE_TXFNF)) {
      }
      USART2->TDR = u.prefix[i];
    }
    u.prefix_n = 0;
    bool sent = false;
    while (u.tx_cb && cb_peek_avail(u.tx_cb) > 0) {
      while (!(USART2->ISR & USART_ISR_TXE_TXFNF)) {
      }
      USART2->TDR = *(const uint8_t *)cb_peek(u.tx_cb);
      cb_skip(u.tx_cb, 1);
      sent = true;
    }
    if (sent) {
      while (!(USART2->ISR & USART_ISR_TC)) {
      }
    }
  } else if (u.tx_n == 0 && !u.prefix_busy) {
    tx_next();
  }
}

void uart_tx_pause(bool pause) {
  u.paused = pause;
}

bool uart_tx_idle(void) {
  return u.tx_n == 0 && !u.prefix_busy;
}

void uart_tx_panic(void) {
  u.panic = true;
  if (u.tx_n > 0 || u.prefix_busy) {
    // Stop the DMA where it is; what it sent is gone from the ring.
    size_t left = TX_CH->CNDTR;
    TX_CH->CCR = 0;
    USART2->CR3 &= ~USART_CR3_DMAT;
    USART2->CR1 &= ~USART_CR1_TCIE;
    if (!u.prefix_busy) cb_skip(u.tx_cb, u.tx_n - left);
    u.tx_n = 0;
    u.prefix_busy = false;
    u.prefix_n = 0;
    while (!(USART2->ISR & USART_ISR_TC)) {
    }
  }
}

size_t uart_rx_avail(void) {
  size_t w = u.rx_cb->n - RX_CH->CNDTR;
  if (w >= u.rx_cb->n) w -= u.rx_cb->n;
  u.rx_cb->write = w;
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
  size_t pos = u.rx_cb->n - RX_CH->CNDTR;
  if (pos >= u.rx_cb->n) pos -= u.rx_cb->n;
  size_t d = pos >= u.rx_pos ? pos - u.rx_pos : pos + u.rx_cb->n - u.rx_pos;
  u.rx_pos = pos;
  u.rx_total += d;
  return u.rx_total;
}

uart_error_t uart_rx_check(void) {
  if (u.error & (UART_ERROR_TX_DMA | UART_ERROR_RX_DMA)) LOG_FATAL("uart: DMA error {enum:uart_error_t}%d", u.error);
  uart_error_t e = u.error;
  if (e == UART_ERROR_NONE) return e;
  u.error = UART_ERROR_NONE;
  (void)USART2->RDR;
  USART2->ICR = USART_ICR_ORECF | USART_ICR_FECF | USART_ICR_NECF;
  USART2->CR1 |= USART_CR1_RE;
  rx_start();
  return e;
}
