// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// STM32WBA55G-DK1: 32 MHz from the HSE (the radio's crystal), LSE for the radio sleep timer,
// SysTick at 1 kHz, user LED LD3 on PB15, and two ucLog links: USART1 at 1 Mbaud (the ST-LINK
// virtual COM port, uart.c) and BLE (ble.c). Implements the ucLog port interface.
//
// One link is in use at a time. A link that receives bytes while the one in use has been silent
// for 2 s (log.c's host timeout: no host there) takes over, so a host can connect on either
// without rebuilding. Bytes received on the link not in use are discarded.
//
// Interrupt priorities (4 bits): the radio 0 (and 5 when only its sleep timer runs), USART1 2,
// SysTick 3, the BLE stack and the link layer's background work 15. board_lock() masks
// everything below the radio through BASEPRI, so the ucLog critical sections never delay it.

#include "board.h"

#include <errno.h>
#include <stddef.h>

#include "main.h" // the example's: HAL, LL and app_conf.h

#include "ble.h"
#include "cb.h"
#include "log.h"
#include "uart.h"

#define BAUD 1000000u
#define HOST_GONE_MS 2000 // log.c's HOST_TIMEOUT_MS
#define LOCK_PRIO 1       // board_lock() masks priorities 1..15

static volatile uint32_t ms;

typedef enum { LINK_UART, LINK_BLE, LINKS } link_t;
static const char *const link_names[LINKS] = {"uart", "ble"};

static struct {
  volatile link_t active;
  link_t rx; // the ring log_poll() reads: latched by log_port_rx_avail() for its peek and skip
  bool paused, panic, started;
  uint32_t count[LINKS], last_rx[LINKS];
  bool heard[LINKS];
} lk;

// The UART RX ring is written by DMA; it holds more than one maximum frame (1452 bytes).
static uint8_t uart_rx_buf[4096];
static cb_t uart_rx_cb = CB_INIT(uart_rx_buf);

// ---------------------------------------------------------------------------------------------
// Board

static void clock_init(void) {
  // As ST's BLE examples: SMPS, voltage scale 1, HSE 32 MHz as SYSCLK (no wait state), LSE on.
  HAL_RCCEx_HSESetTrimming(0x0C);
  HAL_PWREx_ConfigSupply(PWR_SMPS_SUPPLY);
  HAL_PWREx_ControlVoltageScaling(PWR_REGULATOR_VOLTAGE_SCALE1);
  HAL_PWR_EnableBkUpAccess();
  __HAL_RCC_LSEDRIVE_CONFIG(RCC_LSEDRIVE_MEDIUMLOW);
  RCC_OscInitTypeDef osc = {0};
  osc.OscillatorType = RCC_OSCILLATORTYPE_HSE | RCC_OSCILLATORTYPE_LSE;
  osc.HSEState = RCC_HSE_ON;
  osc.HSEDiv = RCC_HSE_DIV1;
  osc.LSEState = RCC_LSE_ON;
  osc.PLL1.PLLState = RCC_PLL_NONE;
  if (HAL_RCC_OscConfig(&osc) != HAL_OK) Error_Handler();
  RCC_ClkInitTypeDef clk = {0};
  clk.ClockType = RCC_CLOCKTYPE_HCLK | RCC_CLOCKTYPE_SYSCLK | RCC_CLOCKTYPE_PCLK1 | RCC_CLOCKTYPE_PCLK2 |
                  RCC_CLOCKTYPE_PCLK7 | RCC_CLOCKTYPE_HCLK5;
  clk.SYSCLKSource = RCC_SYSCLKSOURCE_HSE;
  clk.AHBCLKDivider = RCC_SYSCLK_DIV1;
  clk.APB1CLKDivider = RCC_HCLK_DIV1;
  clk.APB2CLKDivider = RCC_HCLK_DIV1;
  clk.APB7CLKDivider = RCC_HCLK_DIV1;
  clk.AHB5_PLL1_CLKDivider = RCC_SYSCLK_PLL1_DIV1;
  clk.AHB5_HSEHSI_CLKDivider = RCC_SYSCLK_HSEHSI_DIV1;
  if (HAL_RCC_ClockConfig(&clk, FLASH_LATENCY_0) != HAL_OK) Error_Handler();
  RCC_PeriphCLKInitTypeDef periph = {0};
  periph.PeriphClockSelection = RCC_PERIPHCLK_RADIOST;
  periph.RadioSlpTimClockSelection = RCC_RADIOSTCLKSOURCE_LSE;
  if (HAL_RCCEx_PeriphCLKConfig(&periph) != HAL_OK) Error_Handler();
}

// Called by HAL_Init(): the PWR clock (for the supply and voltage scaling settings in
// clock_init(); the radio needs voltage range 1) and the radio interrupt, as ST's examples.
void HAL_MspInit(void) {
  __HAL_RCC_PWR_CLK_ENABLE();
  HAL_NVIC_SetPriority(RADIO_IRQn, RADIO_INTR_PRIO_LOW, 0);
  HAL_NVIC_EnableIRQ(RADIO_IRQn);
}

// HAL time base: board_ms(), from our SysTick (HAL_Init() must not reprogram it).
HAL_StatusTypeDef HAL_InitTick(uint32_t TickPriority) {
  (void)TickPriority;
  return HAL_OK;
}

uint32_t HAL_GetTick(void) {
  return ms;
}

void Error_Handler(void) {
  LOG_FATAL("HAL error");
}

void board_init(void) {
  HAL_Init();
  NVIC_SetPriorityGrouping(3); // 4 bits of preemption priority, as HAL_Init() does
  // SysTick runs during clock_init() too (at the reset clock), so that the HAL's timeouts for the
  // oscillators expire and reach Error_Handler() instead of hanging.
  SysTick_Config(SystemCoreClock / 1000);
  NVIC_SetPriority(SysTick_IRQn, 3);
  clock_init();
  SysTick_Config(SystemCoreClock / 1000);
  NVIC_SetPriority(SysTick_IRQn, 3);
  LL_ICACHE_SetMode(LL_ICACHE_1WAY);
  LL_ICACHE_Enable();
  RCC->AHB2ENR |= RCC_AHB2ENR_GPIOBEN;
  GPIOB->MODER = (GPIOB->MODER & ~GPIO_MODER_MODE15) | GPIO_MODER_MODE15_0;
}

void board_led(bool on) {
  if (on) GPIOB->BSRR = GPIO_BSRR_BS15;
  else GPIOB->BRR = GPIO_BRR_BR15;
}

void board_led_toggle(void) {
  board_led(!(GPIOB->ODR & GPIO_ODR_OD15));
}

const uint8_t *board_uid(void) {
  return (const uint8_t *)UID_BASE;
}

// DWT's cycle counter, started on first use.
uint32_t board_cycles(void) {
  if (!(DWT->CTRL & DWT_CTRL_CYCCNTENA_Msk)) {
    CoreDebug->DEMCR |= CoreDebug_DEMCR_TRCENA_Msk;
    DWT->CYCCNT = 0;
    DWT->CTRL |= DWT_CTRL_CYCCNTENA_Msk;
  }
  return DWT->CYCCNT;
}

uint32_t board_hz(void) {
  return SystemCoreClock;
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

bool board_lock(void) {
  bool enabled = __get_BASEPRI() == 0;
  __set_BASEPRI_MAX(LOCK_PRIO << (8 - __NVIC_PRIO_BITS));
  return enabled;
}

void board_unlock(bool enabled) {
  if (enabled) __set_BASEPRI(0);
}

void HardFault_Handler(void) {
  LOG_FATAL("hard fault");
}

void USART1_IRQHandler(void) {
  uart_irq();
}

void TSC_IRQHandler(void) { // BLE_IRQn
  ble_irq();
}

// The radio's interrupts (as ST's examples; the callbacks are set by linklayer_plat.c).
extern void (*radio_callback)(void);
extern void (*low_isr_callback)(void);
extern volatile uint8_t radio_sw_low_isr_is_running_high_prio;

void RADIO_IRQHandler(void) {
  if (radio_callback != NULL) radio_callback();
  LL_RCC_RADIO_DisableSleepTimerClock();
  __ISB();
}

void HASH_IRQHandler(void) { // RADIO_SW_LOW_INTR_NUM
  NVIC_DisableIRQ(RADIO_SW_LOW_INTR_NUM); // no nesting
  if (low_isr_callback != NULL) low_isr_callback();
  if (radio_sw_low_isr_is_running_high_prio != 0) {
    HAL_NVIC_SetPriority((IRQn_Type)RADIO_SW_LOW_INTR_NUM, RADIO_INTR_PRIO_LOW, 0);
    radio_sw_low_isr_is_running_high_prio = 0;
  }
  NVIC_EnableIRQ(RADIO_SW_LOW_INTR_NUM);
}

// ---------------------------------------------------------------------------------------------
// Links

static uint32_t link_count(link_t l) {
  return l == LINK_UART ? uart_rx_count() : ble_rx_count();
}

static void link_discard(link_t l) {
  if (l == LINK_UART) uart_rx_discard();
  else ble_rx_discard();
}

static void link_pause(link_t l, bool pause) {
  if (l == LINK_UART) uart_tx_pause(pause);
  else ble_tx_pause(pause);
}

static bool link_idle(link_t l) {
  return l == LINK_UART ? uart_tx_idle() : ble_tx_idle();
}

// Every millisecond, before log_tick(): note which links hear a host, and hand the log over to
// the other link when the one in use has gone quiet.
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
    bool en = board_lock();
    link_pause(a, true);
    if (a == LINK_BLE) ble_tx_abort();
    if (link_idle(a)) { // a UART transfer in progress finishes first
      link_discard(a);
      link_pause(next, lk.paused);
      lk.active = next;
    }
    board_unlock(en);
    if (lk.active == next) LOG_INFO("link: %s", link_names[next]);
  }
  for (link_t l = 0; l < LINKS; l++) {
    if (l != lk.active) link_discard(l);
  }
}

void SysTick_Handler(void) {
  ms = ms + 1;
  links_tick(ms);
  log_tick(ms);
  ble_tick();
}

// ---------------------------------------------------------------------------------------------
// ucLog port interface (log.h): the link in use

uint32_t log_port_lock(void) {
  return board_lock();
}

void log_port_unlock(uint32_t key) {
  board_unlock(key != 0);
}

uint32_t log_port_ms(void) {
  return ms;
}

void log_port_init(cb_t *tx_cb) {
  uart_tx_attach(tx_cb);
  ble_tx_attach(tx_cb);
  uart_init(BAUD, &uart_rx_cb);
  ble_init(BLE_NAME);
  lk.active = LINK_UART;
  lk.paused = true;
  for (link_t l = 0; l < LINKS; l++) {
    link_pause(l, true);
    lk.count[l] = link_count(l);
  }
  lk.started = true;
}

void log_port_tx_schedule(const uint8_t *prefix, size_t n) {
  bool en = board_lock();
  if (lk.active == LINK_UART) uart_tx_schedule(prefix, n);
  else ble_tx_schedule(prefix, n);
  board_unlock(en);
}

void log_port_tx_pause(bool pause) {
  bool en = board_lock();
  lk.paused = pause;
  link_pause(lk.active, pause);
  board_unlock(en);
}

bool log_port_tx_idle(void) {
  return link_idle(lk.active);
}

void log_port_panic(void) {
  // Interrupts are off from here on: only the UART can still send (polled).
  lk.panic = true;
  if (lk.active == LINK_UART) uart_tx_panic();
}

size_t log_port_rx_avail(void) {
  // SysTick may hand over to the other link between this call and the skip that follows it.
  lk.rx = lk.active;
  return lk.rx == LINK_UART ? uart_rx_avail() : ble_rx_avail();
}

const uint8_t *log_port_rx_peek(void) {
  return lk.rx == LINK_UART ? uart_rx_peek() : ble_rx_peek();
}

void log_port_rx_skip(size_t n) {
  if (lk.rx == LINK_UART) uart_rx_skip(n);
  else ble_rx_skip(n);
}

uint32_t log_port_rx_mark(void) {
  // Changes whenever the link in use receives a byte, and when the other link takes over.
  return lk.count[lk.active] + ((uint32_t)lk.active << 28);
}

uint32_t log_port_rx_check(void) {
  // Bytes lost (UART errors, a full RX ring): log.c resynchronises on the next frame.
  uart_error_t e = uart_rx_check();
  bool ble_lost = ble_rx_overflow();
  if (lk.active == LINK_UART) return (uint32_t)e;
  return ble_lost ? 1 : 0;
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
