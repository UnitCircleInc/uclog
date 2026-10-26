// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// NUCLEO-C071RB: 48 MHz from HSIUSB48, SysTick at 1 kHz, user LED LD1 on PA5, and three ucLog
// links: USART2 at 1 Mbaud (the ST-LINK virtual COM port, uart.c), and USB CDC-ACM and HID on the
// user USB connector (usb.c). Implements the ucLog port interface.
//
// One link is in use at a time. A link that receives bytes while the one in use has been silent
// for 2 s (log.c's host timeout: no host there) takes over, so a host can connect on any of them
// without rebuilding. Links not in use have their received bytes discarded.

#include "board.h"

#include <errno.h>
#include <stddef.h>

#include <stm32c0xx.h>

#include "cb.h"
#include "log.h"
#include "uart.h"
#include "usb.h"

#define BAUD 1000000u
#define HOST_GONE_MS 2000 // log.c's HOST_TIMEOUT_MS

static volatile uint32_t ms;

typedef enum { LINK_UART, LINK_CDC, LINK_HID, LINKS } link_t;
static const char *const link_names[LINKS] = {"uart", "cdc", "hid"};

static struct {
  volatile link_t active;
  link_t rx; // the ring log_poll() reads: latched by log_port_rx_avail() for its peek and skip
  bool paused, panic, started;
  uint32_t count[LINKS], last_rx[LINKS];
  bool heard[LINKS];
} lk;

// The UART RX ring is written by DMA; it holds more than one maximum frame (1452 bytes).
static uint8_t uart_rx_buf[2048];
static cb_t uart_rx_cb = CB_INIT(uart_rx_buf);

// ---------------------------------------------------------------------------------------------
// Board

static void clock_init(void) {
  // SYSCLK = HSIUSB48 (48 MHz, also the USB clock): one flash wait state.
  FLASH->ACR = (FLASH->ACR & ~FLASH_ACR_LATENCY) | FLASH_ACR_LATENCY_0 | FLASH_ACR_PRFTEN;
  while ((FLASH->ACR & FLASH_ACR_LATENCY) != FLASH_ACR_LATENCY_0) {
  }
  RCC->CR |= RCC_CR_HSIUSB48ON;
  while (!(RCC->CR & RCC_CR_HSIUSB48RDY)) {
  }
  RCC->CFGR = (RCC->CFGR & ~RCC_CFGR_SW) | RCC_CFGR_SW_1; // HSIUSB48
  while ((RCC->CFGR & RCC_CFGR_SWS) != RCC_CFGR_SWS_1) {
  }
  SystemCoreClock = 48000000u; // SystemCoreClockUpdate() does not know HSIUSB48
}

void board_init(void) {
  clock_init();
  SysTick_Config(SystemCoreClock / 1000);
  NVIC_SetPriority(SysTick_IRQn, 2); // below the UART, DMA (0) and USB (1)
  RCC->IOPENR |= RCC_IOPENR_GPIOAEN;
  GPIOA->MODER = (GPIOA->MODER & ~GPIO_MODER_MODE5) | GPIO_MODER_MODE5_0;
}

void board_led(bool on) {
  if (on) GPIOA->BSRR = GPIO_BSRR_BS5;
  else GPIOA->BRR = GPIO_BRR_BR5;
}

void board_led_toggle(void) {
  board_led(!(GPIOA->ODR & GPIO_ODR_OD5));
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

const char *board_link(void) {
  return link_names[lk.active];
}

bool disable_irq(void) {
  bool enabled = __get_PRIMASK() == 0;
  __disable_irq();
  return enabled;
}

void enable_irq(bool enabled) {
  if (enabled) __enable_irq();
}

void HardFault_Handler(void) {
  LOG_FATAL("hard fault");
}

void USART2_IRQHandler(void) {
  uart_irq();
}

void DMA1_Channel1_IRQHandler(void) {
  uart_dma_irq();
}

void DMA1_Channel2_3_IRQHandler(void) {
  uart_dma_irq();
}

void USB_DRD_FS_IRQHandler(void) {
  usb_irq();
}

// ---------------------------------------------------------------------------------------------
// Links

static uint32_t link_count(link_t l) {
  switch (l) {
  case LINK_UART:
    return uart_rx_count();
  case LINK_CDC:
    return usb_rx_count(USB_FN_CDC);
  default:
    return usb_rx_count(USB_FN_HID);
  }
}

static void link_discard(link_t l) {
  if (l == LINK_UART) uart_rx_discard();
  else usb_rx_discard(l == LINK_CDC ? USB_FN_CDC : USB_FN_HID);
}

static void link_pause(link_t l, bool pause) {
  if (l == LINK_UART) uart_tx_pause(pause);
  else usb_tx_pause(l == LINK_CDC ? USB_FN_CDC : USB_FN_HID, pause);
}

static bool link_idle(link_t l) {
  return l == LINK_UART ? uart_tx_idle() : usb_tx_idle(l == LINK_CDC ? USB_FN_CDC : USB_FN_HID);
}

// Every millisecond, before log_tick(): note which links hear a host, and hand the log over to
// another link when the one in use has gone quiet.
static void links_tick(uint32_t now) {
  if (!lk.started || lk.panic) return;
  for (link_t l = 0; l < LINKS; l++) {
    uint32_t c = link_count(l);
    if (c != lk.count[l]) {
      lk.count[l] = c;
      lk.last_rx[l] = now;
      lk.heard[l] = true;
    }
  }
  link_t a = lk.active;
  bool quiet = !lk.heard[a] || now - lk.last_rx[a] > HOST_GONE_MS;
  link_t next = a;
  if (quiet) {
    for (link_t l = 0; l < LINKS; l++) {
      if (l != a && lk.heard[l] && now - lk.last_rx[l] < 100) next = l;
    }
  }
  if (next != a) {
    bool en = disable_irq();
    link_pause(a, true);
    if (a != LINK_UART) usb_tx_abort(a == LINK_CDC ? USB_FN_CDC : USB_FN_HID);
    if (link_idle(a)) { // a UART DMA transfer in progress finishes first
      link_discard(a);
      link_pause(next, lk.paused);
      lk.active = next;
    }
    enable_irq(en);
    if (lk.active == next) LOG_INFO("link: %s", link_names[next]);
  }
  for (link_t l = 0; l < LINKS; l++) {
    if (l != lk.active) link_discard(l);
  }
}

void SysTick_Handler(void) {
  ms = ms + 1;
  links_tick(ms);
  bool en = disable_irq();
  usb_tx_tick();
  enable_irq(en);
  log_tick(ms);
}

// ---------------------------------------------------------------------------------------------
// ucLog port interface (log.h): the link in use

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
  uart_tx_attach(tx_cb);
  usb_tx_attach(USB_FN_CDC, tx_cb);
  usb_tx_attach(USB_FN_HID, tx_cb);
  uart_init(BAUD, &uart_rx_cb);
  usb_init(board_uid());
  lk.active = LINK_UART;
  lk.paused = true;
  for (link_t l = 0; l < LINKS; l++) {
    link_pause(l, true);
    lk.count[l] = link_count(l);
  }
  lk.started = true;
}

void log_port_tx_schedule(const uint8_t *prefix, size_t n) {
  bool en = disable_irq();
  switch (lk.active) {
  case LINK_UART:
    uart_tx_schedule(prefix, n);
    break;
  case LINK_CDC:
    usb_tx_schedule(USB_FN_CDC, prefix, n);
    if (lk.panic) usb_tx_flush_polled(USB_FN_CDC, 500);
    break;
  default:
    usb_tx_schedule(USB_FN_HID, prefix, n);
    if (lk.panic) usb_tx_flush_polled(USB_FN_HID, 500);
    break;
  }
  enable_irq(en);
}

void log_port_tx_pause(bool pause) {
  bool en = disable_irq();
  lk.paused = pause;
  link_pause(lk.active, pause);
  // Host gone: take back an armed IN packet (its bytes stay in the log ring). A closed CDC port
  // sends no IN tokens, so it would stay armed and go out, stale and mid-frame, on reopen; and
  // the device library only drops a cut-off frame when the link is idle.
  if (pause && lk.active != LINK_UART) usb_tx_abort(lk.active == LINK_CDC ? USB_FN_CDC : USB_FN_HID);
  enable_irq(en);
}

bool log_port_tx_idle(void) {
  return link_idle(lk.active);
}

void log_port_panic(void) {
  lk.panic = true;
  if (lk.active == LINK_UART) uart_tx_panic();
}

size_t log_port_rx_avail(void) {
  // SysTick may hand over to another link between this call and the skip that follows it.
  lk.rx = lk.active;
  switch (lk.rx) {
  case LINK_UART:
    return uart_rx_avail();
  case LINK_CDC:
    return usb_rx_avail(USB_FN_CDC);
  default:
    return usb_rx_avail(USB_FN_HID);
  }
}

const uint8_t *log_port_rx_peek(void) {
  switch (lk.rx) {
  case LINK_UART:
    return uart_rx_peek();
  case LINK_CDC:
    return usb_rx_peek(USB_FN_CDC);
  default:
    return usb_rx_peek(USB_FN_HID);
  }
}

void log_port_rx_skip(size_t n) {
  switch (lk.rx) {
  case LINK_UART:
    uart_rx_skip(n);
    break;
  case LINK_CDC:
    usb_rx_skip(USB_FN_CDC, n);
    break;
  default:
    usb_rx_skip(USB_FN_HID, n);
    break;
  }
}

uint32_t log_port_rx_mark(void) {
  // Changes whenever the link in use receives a byte, and when another link takes over.
  return lk.count[lk.active] + ((uint32_t)lk.active << 28);
}

uint32_t log_port_rx_check(void) {
  // UART overrun/framing/noise errors: RX restarted; they matter only when the UART is in use.
  uart_error_t e = uart_rx_check();
  return lk.active == LINK_UART ? (uint32_t)e : 0;
}

__attribute__((noreturn)) void log_port_fatal(void) {
  // Cortex-M0+: no DHCSR access from the core; a debugger catches the reset instead.
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
