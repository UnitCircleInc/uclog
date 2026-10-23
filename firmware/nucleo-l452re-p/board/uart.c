// © 2022 Unit Circle Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#include <stdint.h>
#include <stdbool.h>
#include <stm32l4xx_ll_bus.h>
#include <stm32l4xx_ll_rcc.h>
#include <stm32l4xx_ll_usart.h>
#include <stm32l4xx_ll_lpuart.h>
#include <uart.h>
#include <log.h>

// Adapted from the Unit Circle STM32L4 uart.c: no scheduler (wake-up notify is a callback), and
// a TX prefix and TX pause for the ucLog link (log.c).

static void uart_tx_next(uart_t* uart);

void uart_handler(uart_t* uart) {
  // and we don't manage to get to stop mode before character comes in
  // then RXNE is set with the character and we need to read/clear.
  // As it will be the "wakeup" character we can just toss.
  if (LL_USART_IsEnabledIT_RXNE(uart->uart) && LL_USART_IsActiveFlag_RXNE(uart->uart)) {
    // We must have been in WKUP mode - but were slow to get to ISR
    // Read the character and the WKUP handler will re-enable DMA
    LL_USART_DisableIT_RXNE(uart->uart);
    LOG_INFO("rxne %p %02x", (void*) uart->uart, LL_USART_ReceiveData8(uart->uart));
  }
  if (LL_USART_IsActiveFlag_WKUP(uart->uart) &&
      LL_USART_IsEnabledIT_WKUP(uart->uart)) {
    if (((uint32_t) uart->uart) == USART1_BASE) {
      // Restore to SYS CLK to get correct baudrate
      // We don't care about first few characters being in error
      LL_RCC_SetUSARTClockSource(LL_RCC_USART1_CLKSOURCE_SYSCLK);
    }
    LL_USART_EnableDMAReq_RX(uart->uart);
    LL_USART_DisableIT_WKUP(uart->uart);
    LL_USART_ClearFlag_WKUP(uart->uart);
    LL_USART_DisableInStopMode(uart->uart);
    if (uart->notify) uart->notify(uart->notify_ctx);
  }
  if (LL_USART_IsEnabledIT_TC(uart->uart) &&
      LL_USART_IsActiveFlag_TC(uart->uart)) {
    LL_USART_ClearFlag_TC(uart->uart);
    LL_USART_DisableIT_TC(uart->uart);

    if (uart->tx_prefix_busy) {
      uart->tx_prefix_busy = false;
      uart->tx_prefix_n = 0;
    }
    else {
      cb_skip(uart->tx_cb, uart->tx_n);
    }
    uart->tx_n = 0;
    uart_tx_next(uart);
  }
  uint32_t errors = LL_USART_ReadReg(uart->uart, ISR) &
                (USART_ISR_ORE | USART_ISR_FE | USART_ISR_NE);
  if ((0 != errors) && LL_USART_IsEnabledIT_ERROR(uart->uart)) {
    //LOG_ERROR("uart isr error %08lx", errors);
    LL_USART_DisableIT_ERROR(uart->uart);
    LL_USART_DisableDirectionRx(uart->uart);
    LL_USART_DisableDMAReq_RX(uart->uart);
    dma_stop(uart->rx_dma);
    if ((errors & USART_ISR_ORE) != 0) {
      uart->error |= UART_ERROR_OR;
      LL_USART_ClearFlag_ORE(uart->uart);
    }
    if ((errors & USART_ISR_FE) != 0) {
      uart->error |= UART_ERROR_FRAMING;
      LL_USART_ClearFlag_FE(uart->uart);
    }
    if ((errors & USART_ISR_NE) != 0) {
      uart->error |= UART_ERROR_NOISE;
      LL_USART_ClearFlag_NE(uart->uart);
    }
  }
}

static void uart_tx_dma_complete(void* ctx) {
  uart_t* uart = ctx;
  LL_USART_DisableDMAReq_TX(uart->uart);
  dma_stop(uart->tx_dma);
  LL_USART_EnableIT_TC(uart->uart);
}

static void uart_tx_dma_error(void* ctx) {
  uart_t* uart = ctx;
  LL_USART_DisableDMAReq_TX(uart->uart);
  dma_stop(uart->tx_dma);
  uart->error |= UART_ERROR_TX_DMA;
}

static void uart_rx_dma_error(void* ctx) {
  uart_t* uart = ctx;
  LL_USART_DisableIT_ERROR(uart->uart);
  LL_USART_DisableDMAReq_RX(uart->uart);
  dma_stop(uart->rx_dma);
  uart->error |= UART_ERROR_RX_DMA;
}

void uart_on_wakeup_notify(uart_t* uart, void (*notify)(void* ctx), void* ctx) {
  uart->notify_ctx = ctx;
  uart->notify = notify;
}

void uart_init(uart_t* uart) {
  LOG_INFO("uart init %p", (void*) uart->uart);
  uart->notify = NULL;
  uart->tx_n = 0;
  uart->tx_prefix = NULL;
  uart->tx_prefix_n = 0;
  uart->tx_prefix_busy = false;
  uart->tx_paused = false;
  uart->error_noise_count = 0;
  uart->error_frame_count = 0;
  uart->panic_mode = false;
  cb_reset(uart->tx_cb);
  cb_reset(uart->rx_cb);
  switch ((uint32_t) uart->uart) {
#if defined(USART1)
    case USART1_BASE:
      //LL_RCC_SetUSARTClockSource(LL_RCC_USART1_CLKSOURCE_HSI);
      LL_RCC_SetUSARTClockSource(LL_RCC_USART1_CLKSOURCE_SYSCLK);
      LL_APB2_GRP1_EnableClock(LL_APB2_GRP1_PERIPH_USART1);
      LL_APB2_GRP1_ForceReset(LL_APB2_GRP1_PERIPH_USART1);
      LL_APB2_GRP1_ReleaseReset(LL_APB2_GRP1_PERIPH_USART1);
      LL_USART_Disable(uart->uart);
      NVIC_SetPriority(USART1_IRQn, 0);
      NVIC_EnableIRQ(USART1_IRQn);
      break;
#endif
#if defined(USART2)
    case USART2_BASE:
      LL_RCC_SetUSARTClockSource(LL_RCC_USART2_CLKSOURCE_HSI);
      LL_APB1_GRP1_EnableClock(LL_APB1_GRP1_PERIPH_USART2);
      LL_APB1_GRP1_ForceReset(LL_APB1_GRP1_PERIPH_USART2);
      LL_APB1_GRP1_ReleaseReset(LL_APB1_GRP1_PERIPH_USART2);
      LL_USART_Disable(uart->uart);
      NVIC_SetPriority(USART2_IRQn, 0);
      NVIC_EnableIRQ(USART2_IRQn);
      break;
#endif
#if defined(USART3)
    case USART3_BASE:
      LL_RCC_SetUSARTClockSource(LL_RCC_USART3_CLKSOURCE_HSI);
      LL_APB1_GRP1_EnableClock(LL_APB1_GRP1_PERIPH_USART3);
      LL_APB1_GRP1_ForceReset(LL_APB1_GRP1_PERIPH_USART3);
      LL_APB1_GRP1_ReleaseReset(LL_APB1_GRP1_PERIPH_USART3);
      LL_USART_Disable(uart->uart);
      NVIC_SetPriority(USART3_IRQn, 0);
      NVIC_EnableIRQ(USART3_IRQn);
      break;
#endif
#if defined(UART4)
    case UART4_BASE:
      LL_RCC_SetUSARTClockSource(LL_RCC_UART4_CLKSOURCE_HSI);
      LL_APB1_GRP1_EnableClock(LL_APB1_GRP1_PERIPH_UART4);
      LL_APB1_GRP1_ForceReset(LL_APB1_GRP1_PERIPH_UART4);
      LL_APB1_GRP1_ReleaseReset(LL_APB1_GRP1_PERIPH_UART4);
      LL_USART_Disable(uart->uart);
      NVIC_SetPriority(UART4_IRQn, 0);
      NVIC_EnableIRQ(UART4_IRQn);
      break;
#endif
#if defined(LPUART1)
    case LPUART1_BASE:
      LL_RCC_SetUSARTClockSource(LL_RCC_LPUART1_CLKSOURCE_HSI);
      LL_APB1_GRP2_EnableClock(LL_APB1_GRP2_PERIPH_LPUART1);
      LL_APB1_GRP2_ForceReset(LL_APB1_GRP2_PERIPH_LPUART1);
      LL_APB1_GRP2_ReleaseReset(LL_APB1_GRP2_PERIPH_LPUART1);
      LL_USART_Disable(uart->uart);
      NVIC_SetPriority(LPUART1_IRQn, 0);
      NVIC_EnableIRQ(LPUART1_IRQn);
      break;
#endif
    default: for(;;) __NOP();
  }

  uint32_t dir = LL_USART_DIRECTION_NONE;
  uint32_t flow = LL_USART_HWCONTROL_NONE;
  if (uart->tx) {
    gpio_enable_clk(uart->tx); gpio_alternate(uart->tx);
    dma_init(uart->tx_dma, uart_tx_dma_complete, uart_tx_dma_error, uart);
    dir |= LL_USART_DIRECTION_TX;
    if (uart->de) {
      gpio_enable_clk(uart->de); gpio_alternate(uart->de);
      LL_USART_SetDESignalPolarity(uart->uart, LL_USART_DE_POLARITY_HIGH);
      LL_USART_SetDEDeassertionTime(uart->uart, 31);
      LL_USART_SetDEAssertionTime(uart->uart, 31);
      LL_USART_EnableDEMode(uart->uart);
    }
    if (uart->cts) {
      gpio_enable_clk(uart->cts); gpio_alternate(uart->cts);
      flow |= LL_USART_HWCONTROL_CTS;
    }
  }
  if (uart->rx) {
    gpio_enable_clk(uart->rx);
    gpio_alternate(uart->rx);
    dma_init(uart->rx_dma, NULL, uart_rx_dma_error, uart);
    dir |= LL_USART_DIRECTION_RX;
    if (uart->re) {
      gpio_enable_clk(uart->re); gpio_output(uart->re);
    }
    if (uart->rts) {
      gpio_enable_clk(uart->rts); gpio_alternate(uart->rts);
      flow |= LL_USART_HWCONTROL_RTS;
    }
  }

  LL_USART_SetHWFlowCtrl(uart->uart, flow);
  LL_USART_SetTransferDirection(uart->uart, dir);
  LL_USART_ConfigCharacter(uart->uart, LL_USART_DATAWIDTH_8B,
                                       LL_USART_PARITY_NONE, LL_USART_STOPBITS_1);
  LL_USART_SetOverSampling(uart->uart, LL_USART_OVERSAMPLING_16);
  LL_USART_EnableOneBitSamp(uart->uart);

  if (((uint32_t) uart->uart) == LPUART1_BASE) {
    LL_LPUART_SetBaudRate(uart->uart, HSI_VALUE, uart->baud);
  }
  else if (((uint32_t) uart->uart) == USART1_BASE) {
    LL_USART_SetOverSampling(uart->uart, LL_USART_OVERSAMPLING_8);
    LL_USART_SetBaudRate(uart->uart, SystemCoreClock, LL_USART_OVERSAMPLING_8, uart->baud);
  }
  else {
    LL_USART_SetBaudRate(uart->uart, HSI_VALUE, LL_USART_OVERSAMPLING_16, uart->baud);
  }
  LL_USART_SetWKUPType(uart->uart, LL_USART_WAKEUP_ON_STARTBIT);
  //LL_USART_SetWKUPType(uart->uart, LL_USART_WAKEUP_ON_RXNE);
  LL_RCC_SetClkAfterWakeFromStop(LL_RCC_STOP_WAKEUPCLOCK_MSI);

  LL_USART_Enable(uart->uart);

  if (uart->tx != NULL) {
    while(!(LL_USART_IsActiveFlag_TEACK(uart->uart))) __NOP();
  }

  if (uart->rx != NULL) {
    while(!(LL_USART_IsActiveFlag_REACK(uart->uart))) __NOP();
    dma_start(uart->rx_dma, uart->rx_cb->b, uart->rx_cb->n);
    LL_USART_EnableDMAReq_RX(uart->uart);
    LL_USART_EnableIT_ERROR(uart->uart);
  }
}

void uart_error_counts(uart_t* uart, uint32_t* noise, uint32_t* frame) {
  *noise = uart->error_noise_count;
  *frame = uart->error_frame_count;
}

void uart_restart_rx(uart_t* uart) {
  // We can restart on rx errors
  if (uart->rx == NULL) LOG_FATAL("rx not configured");
  if (uart->error == UART_ERROR_NONE) LOG_FATAL("restart with no error");

  while (true) {
    size_t n = cb_peek_avail(uart->rx_cb);
    if (n == 0) break;
    LOG_MEM_INFO("rx:", cb_peek(uart->rx_cb), n);
    cb_skip(uart->rx_cb, n);
  }

  // Count non-fatal errors
  if (uart->error & UART_ERROR_NOISE) {
    uart->error_noise_count += 1;
    uart-> error &= ~UART_ERROR_NOISE;
  }
  if (uart->error & UART_ERROR_FRAMING) {
    uart->error_frame_count += 1;
    uart-> error &= ~UART_ERROR_FRAMING;
  }

  // See if any fatal errors remain
  if (uart->error != UART_ERROR_NONE) {
    // uclog-clientv2: {enum:} (was {flags:}, which the spec does not define)
    LOG_FATAL("uart %p error: {enum:uart_error_t}%d",
       (void*) uart->uart, uart->error);
  }
  cb_reset(uart->rx_cb);
  (void) LL_USART_ReceiveData8(uart->uart);
  LL_USART_ClearFlag_ORE(uart->uart);
  LL_USART_ClearFlag_FE(uart->uart);
  LL_USART_ClearFlag_NE(uart->uart);
  LL_USART_EnableDirectionRx(uart->uart);
  while(!(LL_USART_IsActiveFlag_REACK(uart->uart))) __NOP();
  dma_start(uart->rx_dma, uart->rx_cb->b, uart->rx_cb->n);
  LL_USART_EnableDMAReq_RX(uart->uart);
  LL_USART_EnableIT_ERROR(uart->uart);
}

void uart_deinit(uart_t* uart) {
  LOG_INFO("uart deinit %p", (void*) uart->uart);
  if (uart->tx != NULL) {
    LL_USART_DisableDMAReq_TX(uart->uart);
    dma_stop(uart->tx_dma);
  }
  if (uart->rx != NULL) {
    LL_USART_DisableIT_ERROR(uart->uart);
    LL_USART_DisableDMAReq_RX(uart->uart);
    dma_stop(uart->rx_dma);
  }
  LL_USART_Disable(uart->uart);

  if (uart->tx) gpio_analog((gpio_analog_t*) uart->tx);
  if (uart->rx) gpio_analog((gpio_analog_t*) uart->rx);
  if (uart->rts) gpio_analog((gpio_analog_t*) uart->rts);
  if (uart->cts) gpio_analog((gpio_analog_t*) uart->cts);
  if (uart->de) gpio_analog((gpio_analog_t*) uart->de);
  if (uart->re) gpio_analog((gpio_analog_t*) uart->re);

  switch ((uint32_t) uart->uart) {
#if defined(USART1)
    case USART1_BASE:
      NVIC_DisableIRQ(USART1_IRQn);
      LL_APB2_GRP1_ForceReset(LL_APB2_GRP1_PERIPH_USART1);
      LL_APB2_GRP1_ReleaseReset(LL_APB2_GRP1_PERIPH_USART1);
      LL_APB2_GRP1_DisableClock(LL_APB2_GRP1_PERIPH_USART1);
      break;
#endif
#if defined(USART2)
    case USART2_BASE:
      NVIC_DisableIRQ(USART2_IRQn);
      LL_APB1_GRP1_ForceReset(LL_APB1_GRP1_PERIPH_USART2);
      LL_APB1_GRP1_ReleaseReset(LL_APB1_GRP1_PERIPH_USART2);
      LL_APB1_GRP1_DisableClock(LL_APB1_GRP1_PERIPH_USART2);
      break;
#endif
#if defined(USART3)
    case USART3_BASE:
      NVIC_DisableIRQ(USART3_IRQn);
      LL_APB1_GRP1_ForceReset(LL_APB1_GRP1_PERIPH_USART3);
      LL_APB1_GRP1_ReleaseReset(LL_APB1_GRP1_PERIPH_USART3);
      LL_APB1_GRP1_DisableClock(LL_APB1_GRP1_PERIPH_USART3);
      break;
#endif
#if defined(UART4)
    case UART4_BASE:
      NVIC_DisableIRQ(UART4_IRQn);
      LL_APB1_GRP1_ForceReset(LL_APB1_GRP1_PERIPH_UART4);
      LL_APB1_GRP1_ReleaseReset(LL_APB1_GRP1_PERIPH_UART4);
      LL_APB1_GRP1_DisableClock(LL_APB1_GRP1_PERIPH_UART4);
      break;
#endif
#if defined(LPUART1)
    case LPUART1_BASE:
      NVIC_DisableIRQ(LPUART1_IRQn);
      LL_APB1_GRP2_ForceReset(LL_APB1_GRP2_PERIPH_LPUART1);
      LL_APB1_GRP2_ReleaseReset(LL_APB1_GRP2_PERIPH_LPUART1);
      LL_APB1_GRP2_DisableClock(LL_APB1_GRP2_PERIPH_LPUART1);
      break;
#endif
    default: for(;;) __NOP();
  }
}

void uart_set_baud(uart_t* uart, uint32_t baud) {
  if (uart->tx != NULL) {
    LL_USART_DisableDMAReq_TX(uart->uart);
    dma_stop(uart->tx_dma);
  }
  if (uart->rx != NULL) {
    LL_USART_DisableIT_ERROR(uart->uart);
    LL_USART_DisableDMAReq_RX(uart->uart);
    dma_stop(uart->rx_dma);
  }
  LL_USART_Disable(uart->uart);
  if (((uint32_t) uart->uart) == LPUART1_BASE) {
    LL_LPUART_SetBaudRate(uart->uart, HSI_VALUE, uart->baud);
  }
  else {
    LL_USART_SetBaudRate(uart->uart, HSI_VALUE, LL_USART_OVERSAMPLING_16, baud);
  }
  LL_USART_Enable(uart->uart);

  uart->error = 0;
  if (uart->tx != NULL) {
    cb_reset(uart->tx_cb);
    while(!(LL_USART_IsActiveFlag_TEACK(uart->uart))) __NOP();
  }
  if (uart->rx != NULL) {
    cb_reset(uart->rx_cb);
    (void) LL_USART_ReceiveData8(uart->uart);
    LL_USART_ClearFlag_ORE(uart->uart);
    LL_USART_ClearFlag_FE(uart->uart);
    LL_USART_ClearFlag_NE(uart->uart);
    LL_USART_EnableDirectionRx(uart->uart);
    while(!(LL_USART_IsActiveFlag_REACK(uart->uart))) __NOP();
    dma_start(uart->rx_dma, uart->rx_cb->b, uart->rx_cb->n);
    LL_USART_EnableDMAReq_RX(uart->uart);
    LL_USART_EnableIT_ERROR(uart->uart);
  }
}

void uart_tx_schedule(uart_t* uart) {
  if (uart->panic_mode) {
    // Busy wait all output
    bool sent = false;
    while(cb_peek_avail(uart->tx_cb) > 0) {
      while (!LL_USART_IsActiveFlag_TXE(uart->uart)) __NOP();
      LL_USART_TransmitData8(uart->uart, *(uint8_t*)cb_peek(uart->tx_cb));
      cb_skip(uart->tx_cb, 1);
      sent = true;
    }
    // uclog-clientv2: wait for TC only if something was sent; TC stays clear otherwise (it was
    // cleared by the previous flush), so a flush with nothing to send never returned.
    if (sent) {
      while (!LL_USART_IsActiveFlag_TC(uart->uart)) __NOP();
      LL_USART_ClearFlag_TC(uart->uart);
    }
  }
  else if (uart->tx_n == 0 && !uart->tx_prefix_busy) {
    uart_tx_next(uart);
  }
}

// Start the next TX DMA (prefix first), unless paused. Called with no transfer in progress,
// from the TC interrupt or with interrupts disabled.
static void uart_tx_next(uart_t* uart) {
  if (uart->tx_paused) return;
  if (uart->tx_prefix_n > 0) {
    uart->tx_prefix_busy = true;
    dma_start(uart->tx_dma, uart->tx_prefix, uart->tx_prefix_n);
    LL_USART_EnableDMAReq_TX(uart->uart);
    return;
  }
  uart->tx_n = cb_peek_avail(uart->tx_cb);
  if (uart->tx_n > 0) {
    dma_start(uart->tx_dma, cb_peek(uart->tx_cb), uart->tx_n);
    LL_USART_EnableDMAReq_TX(uart->uart);
  }
}

void uart_tx_schedule_prefix(uart_t* uart, const uint8_t* prefix, size_t n) {
  uart->tx_prefix = prefix;
  uart->tx_prefix_n = n;
  uart_tx_schedule(uart);
}

void uart_tx_pause(uart_t* uart, bool pause) {
  uart->tx_paused = pause;
}

void uart_tx_panic(uart_t* uart) {
  uart->panic_mode = true;
  // Shut things down DMA/INTERRUPTs if we are in mid transfer
  if (uart->tx_n > 0 || uart->tx_prefix_busy) {
    if (LL_USART_IsEnabledDMAReq_TX(uart->uart)) {
      LL_USART_DisableDMAReq_TX(uart->uart);
      size_t n = uart->tx_n - dma_get_length(uart->tx_dma);
      if (!uart->tx_prefix_busy) cb_skip(uart->tx_cb, n);
    }
    uart->tx_n = 0;
    uart->tx_prefix_busy = false;
    LL_USART_DisableIT_TC(uart->uart);

    // Wait for everything to flush out
    while (!LL_USART_IsActiveFlag_TC(uart->uart)) __NOP();
    LL_USART_ClearFlag_TC(uart->uart);
  }
}

void uart_tx_buffer(uart_t* uart, const uint8_t* b, size_t n) {
  cb_write(uart->tx_cb, b, n);
}

void uart_tx(uart_t* uart, const uint8_t* b, size_t n) {
  uart_tx_buffer(uart, b, n);
  uart_tx_schedule(uart);
}

size_t uart_tx_avail(uart_t* uart) {
  return cb_write_avail(uart->tx_cb);
}

bool uart_tx_done(uart_t* uart) {
  return cb_peek_avail(uart->tx_cb) == 0;
}

//static uint32_t last_dma  = 0;
size_t uart_rx_avail(uart_t* uart) {
  size_t w = uart->rx_cb->n - dma_get_length(uart->rx_dma);
  if (w >= uart->rx_cb->n) w -= uart->rx_cb->n;
  uart->rx_cb->write = w;
  return cb_peek_avail(uart->rx_cb);
}

const uint8_t* uart_rx_peek(uart_t* uart) {
  return cb_peek(uart->rx_cb);
}

void uart_rx_skip(uart_t* uart, size_t n) {
  cb_skip(uart->rx_cb, n);
}

bool uart_lp_enter(uart_t* uart) {
  if (uart->notify == NULL) LOG_FATAL("Trying to enter LP mode with no one to notify");
  // If busy is not set and we use wake up on RXNE
  // then we have at least 9 bits of time to set everything up
  // code is small and interrupts disabled.
  // So should be able to enter wihtout issues.
  // If we don't enter stop mode we will just end up re-enabling
  // DMA and normal reception and HSI will not have turned off so
  // should be no loss of data.  Exception might be if other ISR
  // is running and introduces 1 character time latency we might
  // get a receiver overrun error - presumably this is handled by higher
  // level protocols - e.g.
  // - line has to be idle for 1s before entering LP rx
  // - intiator at other end then sends 1 framing character then pauses
  //   10ms before sending frames.
  // Note: If we get Rx interrupt before going into stop then
  // we will get a RXNE event which needs to be handled in the UART ISR.
  bool disabled = disable_irq();

  if (((uint32_t) uart->uart) == USART1_BASE) {
    // Revert to HSI so we get wakeup behaviour
    // We don't care about first few characters being in error
    LL_RCC_SetUSARTClockSource(LL_RCC_USART1_CLKSOURCE_HSI);
  }


  bool r = false;
  if (uart_rx_avail(uart) == 0) {
    if (LL_USART_IsActiveFlag_BUSY(uart->uart) == 0) {
      LL_USART_DisableDMAReq_RX(uart->uart);
      LL_USART_EnableIT_RXNE(uart->uart); // In case we don't make it to stop

      // Ensure any new transfers result in a wakeup
      LL_USART_ClearFlag_WKUP(uart->uart);
      LL_USART_EnableIT_WKUP(uart->uart);
      LL_USART_EnableInStopMode(uart->uart);
      r = true;
    }
  }
  enable_irq(disabled);
  return r;
}

bool uart_cts_active(uart_t* uart) {
  if (uart->cts) return LL_USART_IsActiveFlag_CTS(uart->uart) != 0;
  return true;
}

void uart_lp_exit(uart_t* uart) {
  // Everything is currently handled in ISR
  (void) uart;
}

void uart_rx_start(uart_t* uart) {
  if (uart->re) {
    gpio_out(uart->re, 0);
    timer_delay_us(10);
  }
}

void uart_rx_stop(uart_t* uart) {
  if (uart->re) gpio_out(uart->re, 1);
}


void uart_resume(uart_t* uart) {
  if (uart->re) {
    gpio_out(uart->re, 0);
    timer_delay_us(10);
  }

  if (uart->rx != NULL) {
    cb_reset(uart->rx_cb);
    (void) LL_USART_ReceiveData8(uart->uart);
    LL_USART_ClearFlag_ORE(uart->uart);
    LL_USART_ClearFlag_FE(uart->uart);
    LL_USART_ClearFlag_NE(uart->uart);
    dma_start(uart->rx_dma, uart->rx_cb->b, uart->rx_cb->n);
    LL_USART_EnableDMAReq_RX(uart->uart);
    LL_USART_EnableIT_ERROR(uart->uart);
  }
}

// This API only used for USART3 comm to sensor board
// So we hack it for the moment to allow faster baud rates
void uart_resume_set_baud(uart_t* uart, uint32_t baud) {
  if (uart->tx != NULL) {
    LL_USART_DisableDMAReq_TX(uart->uart);
    dma_stop(uart->tx_dma);
  }
  uart->error = 0;

  LL_USART_Disable(uart->uart);

  LL_RCC_SetUSARTClockSource(LL_RCC_USART3_CLKSOURCE_SYSCLK);
  LL_USART_SetBaudRate(uart->uart, SystemCoreClock, LL_USART_OVERSAMPLING_16, baud);

  LL_USART_Enable(uart->uart);

  if (uart->tx != NULL) {
    cb_reset(uart->tx_cb);
    while(!(LL_USART_IsActiveFlag_TEACK(uart->uart))) __NOP();
  }


  if (uart->re) {
    gpio_out(uart->re, 0);
    timer_delay_us(10);
  }

  if (uart->rx != NULL) {
    cb_reset(uart->rx_cb);
    (void) LL_USART_ReceiveData8(uart->uart);
    LL_USART_ClearFlag_ORE(uart->uart);
    LL_USART_ClearFlag_FE(uart->uart);
    LL_USART_ClearFlag_NE(uart->uart);
    dma_start(uart->rx_dma, uart->rx_cb->b, uart->rx_cb->n);
    LL_USART_EnableDMAReq_RX(uart->uart);
    LL_USART_EnableIT_ERROR(uart->uart);
  }
}

void uart_pause(uart_t* uart) {
  if (uart->re) gpio_out(uart->re, 1);

  if (uart->rx != NULL) {
    LL_USART_DisableIT_ERROR(uart->uart);
    LL_USART_DisableDMAReq_RX(uart->uart);
    dma_stop(uart->rx_dma);
  }
}

uart_error_t uart_error(uart_t* uart) {
  // Make DMA errors fatal - the other rx errors are recoverable
  if ((uart->error & (UART_ERROR_TX_DMA | UART_ERROR_RX_DMA)) != 0) {
    LOG_FATAL("uart->error: %08x", uart->error);
  }
  return uart->error;
}
