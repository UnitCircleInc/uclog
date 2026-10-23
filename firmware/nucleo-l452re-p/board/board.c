// NUCLEO-L452RE-P: 80 MHz from HSI16 + PLL, USART2 (PA2/PA3, ST-LINK virtual COM port) at
// 1 Mbaud with DMA (RX: DMA1 channel 6, circular; TX: DMA1 channel 7, linear from the log
// ring buffer), SysTick at 1 kHz, user LED LD2 on PB13. Implements the ucLog port interface.
//
// Startup code and SystemInit() (FPU enable) come from STM32CubeL4.

#include "board.h"

#include <errno.h>
#include <stddef.h>

#include <stm32l4xx.h>
#include <stm32l4xx_ll_bus.h>
#include <stm32l4xx_ll_dma.h>
#include <stm32l4xx_ll_rcc.h>
#include <stm32l4xx_ll_system.h>
#include <stm32l4xx_ll_usart.h>

#include "cb.h"
#include "dma.h"
#include "gpio.h"
#include "log.h"
#include "uart.h"

#define BAUD 1000000u

static volatile uint32_t ms;

// ---------------------------------------------------------------------------------------------
// UART + DMA (the Unit Circle driver configuration style)

static const gpio_alternate_t uart_tx_pin = GPIO_ALTERNATE(A, 2, UP, PUSHPULL, VERY_HIGH, LL_GPIO_AF_7);
static const gpio_alternate_t uart_rx_pin = GPIO_ALTERNATE(A, 3, UP, PUSHPULL, VERY_HIGH, LL_GPIO_AF_7);
static const gpio_output_t led = GPIO_OUTPUT(B, 13, NO, PUSHPULL, LOW, 0);

static dma_t uart_tx_dma = {
    .dma = DMA1,
    .ch = LL_DMA_CHANNEL_7,
    .req = LL_DMA_REQUEST_2, // USART2_TX
    .irq = DMA1_Channel7_IRQn,
    .addr = (uint32_t)&USART2->TDR,
    .trans = LL_DMA_DIRECTION_MEMORY_TO_PERIPH | LL_DMA_MODE_NORMAL | LL_DMA_PERIPH_NOINCREMENT |
             LL_DMA_MEMORY_INCREMENT | LL_DMA_PDATAALIGN_BYTE | LL_DMA_MDATAALIGN_BYTE |
             LL_DMA_PRIORITY_MEDIUM,
};

static dma_t uart_rx_dma = {
    .dma = DMA1,
    .ch = LL_DMA_CHANNEL_6,
    .req = LL_DMA_REQUEST_2, // USART2_RX
    .irq = DMA1_Channel6_IRQn,
    .addr = (uint32_t)&USART2->RDR,
    .trans = LL_DMA_DIRECTION_PERIPH_TO_MEMORY | LL_DMA_MODE_CIRCULAR | LL_DMA_PERIPH_NOINCREMENT |
             LL_DMA_MEMORY_INCREMENT | LL_DMA_PDATAALIGN_BYTE | LL_DMA_MDATAALIGN_BYTE |
             LL_DMA_PRIORITY_HIGH,
};

// The RX ring is written by DMA; it must hold more than one maximum frame (1452 bytes) plus
// whatever arrives while the main loop is busy.
static uint8_t rx_buf[4096];
static cb_t rx_cb = CB_INIT(rx_buf);

static uart_t uart = {
    .uart = USART2,
    .baud = BAUD,
    .tx = &uart_tx_pin,
    .rx = &uart_rx_pin,
    .tx_dma = &uart_tx_dma,
    .rx_dma = &uart_rx_dma,
    .rx_cb = &rx_cb,
    // .tx_cb is the log ring buffer, set by log_port_init()
};

void DMA1_Channel6_IRQHandler(void) {
  dma_ch6_handler(&uart_rx_dma);
}

void DMA1_Channel7_IRQHandler(void) {
  dma_ch7_handler(&uart_tx_dma);
}

void USART2_IRQHandler(void) {
  uart_handler(&uart);
}

// ---------------------------------------------------------------------------------------------
// Board

static void clock_init(void) {
  // PLL: HSI16 / M=1 * N=20 / R=4 = 80 MHz (range 1, 4 wait states)
  LL_RCC_HSI_Enable();
  while (!LL_RCC_HSI_IsReady()) {
  }
  LL_FLASH_SetLatency(LL_FLASH_LATENCY_4);
  while (LL_FLASH_GetLatency() != LL_FLASH_LATENCY_4) {
  }
  LL_FLASH_EnablePrefetch();
  LL_FLASH_EnableInstCache();
  LL_FLASH_EnableDataCache();
  LL_RCC_PLL_ConfigDomain_SYS(LL_RCC_PLLSOURCE_HSI, LL_RCC_PLLM_DIV_1, 20, LL_RCC_PLLR_DIV_4);
  LL_RCC_PLL_EnableDomain_SYS();
  LL_RCC_PLL_Enable();
  while (!LL_RCC_PLL_IsReady()) {
  }
  LL_RCC_SetSysClkSource(LL_RCC_SYS_CLKSOURCE_PLL);
  while (LL_RCC_GetSysClkSource() != LL_RCC_SYS_CLKSOURCE_STATUS_PLL) {
  }
  SystemCoreClockUpdate();
}

void board_init(void) {
  clock_init();
  SysTick_Config(SystemCoreClock / 1000);
  NVIC_SetPriority(SysTick_IRQn, 2); // below the UART and DMA (priority 0)
  gpio_enable_clk(&led);
  gpio_output(&led);
}

void board_led(bool on) {
  gpio_out(&led, on);
}

void board_led_toggle(void) {
  gpio_toggle(&led);
}

const uint8_t *board_uid(void) {
  return (const uint8_t *)UID_BASE;
}

uint32_t board_ms(void) {
  return ms;
}

void board_delay_ms(uint32_t d) {
  uint32_t t = ms;
  while (ms - t < d) {
  }
}

void SysTick_Handler(void) {
  ms = ms + 1;
  log_tick(ms);
}

void HardFault_Handler(void) {
  LOG_FATAL("hard fault");
}

bool disable_irq(void) {
  bool enabled = __get_PRIMASK() == 0;
  __disable_irq();
  return enabled;
}

void enable_irq(bool enabled) {
  if (enabled) __enable_irq();
}

void timer_delay_us(uint32_t us) {
  uint32_t n = us * (SystemCoreClock / 4000000u); // ~4 cycles per iteration
  for (volatile uint32_t i = 0; i < n; i++) {
  }
}

// ---------------------------------------------------------------------------------------------
// ucLog port interface (log.h)

uint32_t log_port_lock(void) {
  return disable_irq();
}

void log_port_unlock(uint32_t key) {
  enable_irq(key != 0);
}

uint32_t log_port_ms(void) {
  return ms;
}

void log_port_init(cb_t *tx_cb) {
  // uart_init() resets its TX buffer: give it an empty one, so the logs already in tx_cb (and
  // its own "uart init" log) are kept.
  static uint8_t none[1];
  static cb_t empty = CB_INIT(none);
  uart.tx_cb = &empty;
  uart_init(&uart);
  uart.tx_cb = tx_cb;
}

void log_port_tx_schedule(const uint8_t *prefix, size_t n) {
  bool en = disable_irq(); // log calls and SysTick may both schedule
  if (prefix != NULL) {
    uart_tx_schedule_prefix(&uart, prefix, n);
  } else {
    uart_tx_schedule(&uart);
  }
  enable_irq(en);
}

void log_port_tx_pause(bool pause) {
  bool en = disable_irq();
  uart_tx_pause(&uart, pause);
  enable_irq(en);
}

bool log_port_tx_idle(void) {
  return uart.tx_n == 0 && !uart.tx_prefix_busy;
}

void log_port_panic(void) {
  uart_tx_panic(&uart);
}

size_t log_port_rx_avail(void) {
  return uart_rx_avail(&uart);
}

const uint8_t *log_port_rx_peek(void) {
  return uart_rx_peek(&uart);
}

void log_port_rx_skip(size_t n) {
  uart_rx_skip(&uart, n);
}

uint32_t log_port_rx_mark(void) {
  return dma_get_length(&uart_rx_dma); // changes with every byte received
}

uint32_t log_port_rx_check(void) {
  // The driver stops RX on overrun/framing/noise errors; count and restart. A DMA error, or an
  // overrun (which DMA should make impossible), is fatal in uart_restart_rx().
  uart_error_t e = uart_error(&uart);
  if (e == UART_ERROR_NONE) return 0;
  uart_restart_rx(&uart);
  return (uint32_t)e;
}

__attribute__((noreturn)) void log_port_fatal(void) {
  if (CoreDebug->DHCSR & CoreDebug_DHCSR_C_DEBUGEN_Msk) __BKPT(0);
  NVIC_SystemReset();
}

// ---------------------------------------------------------------------------------------------
// newlib heap (syscalls.c leaves _sbrk out)

void *_sbrk(ptrdiff_t incr) {
  extern char _end, _heap_limit;
  static char *brk = &_end;
  if (brk + incr > &_heap_limit) {
    errno = ENOMEM;
    return (void *)-1;
  }
  char *prev = brk;
  brk += incr;
  return prev;
}

int _kill(int pid, int sig) {
  (void)pid;
  (void)sig;
  errno = EINVAL;
  return -1;
}
