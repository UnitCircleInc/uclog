// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// The board and the ucLog port (include/log_port.h) for the NUCLEO-L452RE-P, in CMSIS registers
// only: 80 MHz, SysTick at 1 kHz, the user LED, and USART2 (the ST-LINK's virtual COM port,
// PA2/PA3) at 1 Mbaud with DMA:
//
// - RX: DMA1 channel 6 writes a ring buffer, circular, without interrupts. The library reads it
//   with avail/peek/skip; the DMA's remaining count is the write position.
// - TX: DMA1 channel 7 sends the contiguous part of the library's TX ring (or the prefix it asks
//   for first), and its transfer-complete interrupt starts the next part.
//
// A USART without a FIFO (as here) loses bytes at 1 Mbaud if RX is interrupt-driven: use DMA.

#include "port.h"

#include <stm32l4xx.h>

#include "log.h"
#include "log_port.h"

#define BAUD 1000000u

static volatile uint32_t ms;

// ---------------------------------------------------------------------------------------------
// Board

static void clock_init(void) {
  // HSI16 → PLL (M=1, N=20, R=4) = 80 MHz; 4 flash wait states; APB1 and APB2 undivided.
  RCC->CR |= RCC_CR_HSION;
  while (!(RCC->CR & RCC_CR_HSIRDY)) {
  }
  FLASH->ACR = FLASH_ACR_LATENCY_4WS | FLASH_ACR_PRFTEN | FLASH_ACR_ICEN | FLASH_ACR_DCEN;
  RCC->PLLCFGR = RCC_PLLCFGR_PLLSRC_HSI | (0u << RCC_PLLCFGR_PLLM_Pos) | (20u << RCC_PLLCFGR_PLLN_Pos) |
                 (1u << RCC_PLLCFGR_PLLR_Pos) | RCC_PLLCFGR_PLLREN; // PLLR field 1: divide by 4
  RCC->CR |= RCC_CR_PLLON;
  while (!(RCC->CR & RCC_CR_PLLRDY)) {
  }
  RCC->CFGR = (RCC->CFGR & ~RCC_CFGR_SW) | RCC_CFGR_SW_PLL;
  while ((RCC->CFGR & RCC_CFGR_SWS) != RCC_CFGR_SWS_PLL) {
  }
  SystemCoreClockUpdate();
}

void board_init(void) {
  clock_init();
  SysTick_Config(SystemCoreClock / 1000);
  NVIC_SetPriority(SysTick_IRQn, 2); // below the TX DMA interrupt (0)
  RCC->AHB2ENR |= RCC_AHB2ENR_GPIOBEN;
  GPIOB->MODER = (GPIOB->MODER & ~GPIO_MODER_MODE13) | GPIO_MODER_MODE13_0; // LD2: output
}

void board_led(bool on) {
  GPIOB->BSRR = on ? GPIO_BSRR_BS13 : GPIO_BSRR_BR13;
}

uint32_t board_ms(void) {
  return ms;
}

void SysTick_Handler(void) {
  ms = ms + 1;
  log_tick(ms); // pulses and host presence
}

void HardFault_Handler(void) {
  LOG_FATAL("hard fault");
}

// ---------------------------------------------------------------------------------------------
// USART2 and its DMA

static uint8_t rx_buf[2048]; // more than one frame (1452 bytes) plus what arrives meanwhile
static size_t rx_read;

// TX state, shared with the DMA interrupt.
static cb_t *tx_cb;                     // the library's TX ring
static const uint8_t *prefix;           // sent before tx_cb (the device info)
static volatile size_t prefix_n;
static volatile size_t tx_n;            // bytes of the transfer in progress
static volatile bool tx_prefix_busy;    // that transfer is the prefix
static volatile bool tx_paused;
static bool panic;

static void uart_init(void) {
  RCC->AHB1ENR |= RCC_AHB1ENR_DMA1EN;
  RCC->AHB2ENR |= RCC_AHB2ENR_GPIOAEN;
  RCC->APB1ENR1 |= RCC_APB1ENR1_USART2EN;

  // PA2 (TX), PA3 (RX): alternate function 7, very high speed, RX pulled up.
  GPIOA->MODER = (GPIOA->MODER & ~(GPIO_MODER_MODE2 | GPIO_MODER_MODE3)) | GPIO_MODER_MODE2_1 | GPIO_MODER_MODE3_1;
  GPIOA->AFR[0] = (GPIOA->AFR[0] & ~(GPIO_AFRL_AFSEL2 | GPIO_AFRL_AFSEL3)) | (7u << GPIO_AFRL_AFSEL2_Pos) | (7u << GPIO_AFRL_AFSEL3_Pos);
  GPIOA->OSPEEDR |= GPIO_OSPEEDR_OSPEED2 | GPIO_OSPEEDR_OSPEED3;
  GPIOA->PUPDR = (GPIOA->PUPDR & ~GPIO_PUPDR_PUPD3) | GPIO_PUPDR_PUPD3_0;

  // DMA1 request 2 is USART2 on channels 6 (RX) and 7 (TX).
  DMA1_CSELR->CSELR = (DMA1_CSELR->CSELR & ~(DMA_CSELR_C6S | DMA_CSELR_C7S)) | (2u << DMA_CSELR_C6S_Pos) | (2u << DMA_CSELR_C7S_Pos);
  DMA1_Channel6->CPAR = (uint32_t)&USART2->RDR;
  DMA1_Channel6->CMAR = (uint32_t)rx_buf;
  DMA1_Channel6->CNDTR = sizeof(rx_buf);
  DMA1_Channel6->CCR = DMA_CCR_MINC | DMA_CCR_CIRC | DMA_CCR_PL_1 | DMA_CCR_EN;
  DMA1_Channel7->CPAR = (uint32_t)&USART2->TDR;
  DMA1_Channel7->CCR = DMA_CCR_MINC | DMA_CCR_DIR | DMA_CCR_TCIE; // enabled per transfer
  NVIC_SetPriority(DMA1_Channel7_IRQn, 0);
  NVIC_EnableIRQ(DMA1_Channel7_IRQn);

  // 8N1; overrun detection off (DMA keeps up; a lost byte shows as a bad frame, which the library
  // drops and counts).
  USART2->BRR = SystemCoreClock / BAUD; // PCLK1 = SYSCLK
  USART2->CR3 = USART_CR3_DMAR | USART_CR3_DMAT | USART_CR3_OVRDIS;
  USART2->CR1 = USART_CR1_TE | USART_CR1_RE | USART_CR1_UE;
}

static void dma_tx(const void *b, size_t n) {
  DMA1_Channel7->CMAR = (uint32_t)b;
  DMA1_Channel7->CNDTR = n;
  DMA1_Channel7->CCR |= DMA_CCR_EN;
}

// Start the next transfer: the prefix, then what is in tx_cb. With no transfer in progress.
static void tx_next(void) {
  if (tx_paused) return;
  if (prefix_n > 0) {
    tx_prefix_busy = true;
    tx_n = prefix_n;
    dma_tx(prefix, prefix_n);
  } else if ((tx_n = cb_peek_avail(tx_cb)) > 0) {
    dma_tx(cb_peek(tx_cb), tx_n);
  }
}

// The transfer in progress is done: drop what it sent.
static void tx_done(size_t sent) {
  if (tx_prefix_busy) {
    prefix_n = 0;
    tx_prefix_busy = false;
  } else {
    cb_skip(tx_cb, sent);
  }
  tx_n = 0;
}

void DMA1_Channel7_IRQHandler(void) {
  if (DMA1->ISR & DMA_ISR_TCIF7) {
    DMA1->IFCR = DMA_IFCR_CGIF7;
    DMA1_Channel7->CCR &= ~DMA_CCR_EN;
    tx_done(tx_n);
    tx_next();
  }
}

// ---------------------------------------------------------------------------------------------
// The ucLog port

uint32_t log_port_lock(void) {
  uint32_t primask = __get_PRIMASK();
  __disable_irq();
  return primask;
}

void log_port_unlock(uint32_t key) {
  __set_PRIMASK(key);
}

uint32_t log_port_ms(void) {
  return ms;
}

void log_port_init(cb_t *cb) {
  tx_cb = cb;
  uart_init();
}

void log_port_tx_schedule(const uint8_t *p, size_t n) {
  uint32_t key = log_port_lock(); // logs and SysTick both schedule
  if (p != NULL) {
    prefix = p;
    prefix_n = n;
  }
  if (panic) {
    // Polled: interrupts are off for good. Send everything, then wait for the last bit.
    while (cb_peek_avail(tx_cb) > 0) {
      while (!(USART2->ISR & USART_ISR_TXE)) {
      }
      USART2->TDR = *(const uint8_t *)cb_peek(tx_cb);
      cb_skip(tx_cb, 1);
    }
    while (!(USART2->ISR & USART_ISR_TC)) {
    }
  } else if (tx_n == 0) {
    tx_next();
  }
  log_port_unlock(key);
}

void log_port_tx_pause(bool pause) {
  tx_paused = pause;
}

bool log_port_tx_idle(void) {
  return tx_n == 0;
}

void log_port_panic(void) {
  // Stop the DMA where it is and count what it sent; the library then sends the rest, polled.
  panic = true;
  if (tx_n > 0) {
    DMA1_Channel7->CCR &= ~DMA_CCR_EN;
    tx_done(tx_n - DMA1_Channel7->CNDTR);
    prefix_n = 0;
  }
}

static size_t rx_write(void) {
  return sizeof(rx_buf) - DMA1_Channel6->CNDTR;
}

size_t log_port_rx_avail(void) {
  size_t w = rx_write();
  return w >= rx_read ? w - rx_read : sizeof(rx_buf) - rx_read; // contiguous bytes only
}

const uint8_t *log_port_rx_peek(void) {
  return &rx_buf[rx_read];
}

void log_port_rx_skip(size_t n) {
  rx_read = (rx_read + n) % sizeof(rx_buf);
}

uint32_t log_port_rx_mark(void) {
  return DMA1_Channel6->CNDTR; // changes with every byte received
}

uint32_t log_port_rx_check(void) {
  // Framing and noise errors: clear and report them (the bytes still arrive, through the DMA).
  uint32_t e = USART2->ISR & (USART_ISR_FE | USART_ISR_NE);
  if (e) USART2->ICR = USART_ICR_FECF | USART_ICR_NECF;
  return e;
}

__attribute__((noreturn)) void log_port_fatal(void) {
  if (CoreDebug->DHCSR & CoreDebug_DHCSR_C_DEBUGEN_Msk) __BKPT(0);
  NVIC_SystemReset();
}
