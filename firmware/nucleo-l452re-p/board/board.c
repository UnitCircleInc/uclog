// NUCLEO-L452RE-P: 80 MHz from HSI16 + PLL, USART2 (PA2/PA3, ST-LINK virtual COM port) at
// 1 Mbaud, SysTick at 1 kHz, user LED LD2 on PB13. Implements the ucLog port interface.

#include "board.h"

#include <errno.h>
#include <stddef.h>
#include <string.h>

#include "log.h"
#include "stm32l452.h"

#define SYSCLK_HZ 80000000u
#define BAUD 1000000u

static volatile uint32_t ms;

// ---------------------------------------------------------------------------------------------
// Startup

extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss, _estack;
extern int main(void);
extern void __libc_init_array(void);

void Reset_Handler(void) {
  SCB_CPACR |= 0xFu << 20; // CP10/CP11 full access: FPU on before any FP instruction
  __asm volatile("dsb\n isb");
  memcpy(&_sdata, &_sidata, (size_t)((char *)&_edata - (char *)&_sdata));
  memset(&_sbss, 0, (size_t)((char *)&_ebss - (char *)&_sbss));
  __libc_init_array();
  main();
  while (1) {
  }
}

static void Default_Handler(void) {
  LOG_FATAL("unexpected interrupt");
}

void HardFault_Handler(void) {
  LOG_FATAL("hard fault");
}

void SysTick_Handler(void);
void USART2_IRQHandler(void);

// 16 core exceptions + 85 STM32L452 interrupts.
__attribute__((used, section(".isr_vector"))) static void (*const vectors[16 + 85])(void) = {
    [2 ... 16 + 84] = Default_Handler, // NMI and everything not listed below
    [0] = (void (*)(void))&_estack,
    [1] = Reset_Handler,
    [3] = HardFault_Handler,
    [4] = HardFault_Handler, // MemManage
    [5] = HardFault_Handler, // BusFault
    [6] = HardFault_Handler, // UsageFault
    [15] = SysTick_Handler,
    [16 + USART2_IRQn] = USART2_IRQHandler,
};

// ---------------------------------------------------------------------------------------------
// Board

static void clock_init(void) {
  RCC_CR |= RCC_CR_HSION;
  while ((RCC_CR & RCC_CR_HSIRDY) == 0) {
  }
  // PLL: HSI16 / M=1 * N=20 / R=4 = 80 MHz
  RCC_PLLCFGR = (2u << 0)     // PLLSRC = HSI16
              | (0u << 4)     // PLLM = 1
              | (20u << 8)    // PLLN = 20
              | (1u << 24)    // PLLREN
              | (1u << 25);   // PLLR = 4
  RCC_CR |= RCC_CR_PLLON;
  while ((RCC_CR & RCC_CR_PLLRDY) == 0) {
  }
  // 4 wait states at 80 MHz (range 1), prefetch + caches on.
  FLASH_ACR = 4u | (1u << 8) | (1u << 9) | (1u << 10);
  while ((FLASH_ACR & 7u) != 4u) {
  }
  RCC_CFGR = (RCC_CFGR & ~3u) | 3u; // SW = PLL
  while (((RCC_CFGR >> 2) & 3u) != 3u) {
  }
}

static void gpio_init(void) {
  RCC_AHB2ENR |= (1u << 0) | (1u << 1); // GPIOA, GPIOB
  (void)RCC_AHB2ENR;
  // PB13: LED output.
  GPIO_MODER(GPIOB_BASE) = (GPIO_MODER(GPIOB_BASE) & ~(3u << 26)) | (1u << 26);
}

static void uart_init(void) {
  RCC_APB1ENR1 |= 1u << 17; // USART2 (clock = PCLK1 = SYSCLK)
  (void)RCC_APB1ENR1;
  USART2_CR1 = 0;
  USART2_BRR = SYSCLK_HZ / BAUD;
  USART2_CR1 = USART_CR1_TE | USART_CR1_RE | USART_CR1_RXNEIE | USART_CR1_UE;
  // PA2/PA3: AF7 (USART2), pull-ups. Switched over only now that TX idles high, so the host
  // does not see a glitch byte after every reset.
  GPIO_PUPDR(GPIOA_BASE) = (GPIO_PUPDR(GPIOA_BASE) & ~(0xFu << 4)) | (0x5u << 4);
  GPIO_OSPEEDR(GPIOA_BASE) |= 0xFu << 4;
  GPIO_AFRL(GPIOA_BASE) = (GPIO_AFRL(GPIOA_BASE) & ~(0xFFu << 8)) | (0x77u << 8);
  GPIO_MODER(GPIOA_BASE) = (GPIO_MODER(GPIOA_BASE) & ~(0xFu << 4)) | (0xAu << 4);
  NVIC_IPR8(USART2_IRQn) = 0x40;
  NVIC_ISER(USART2_IRQn / 32) = 1u << (USART2_IRQn % 32);
}

void board_init(void) {
  clock_init();
  gpio_init();
  SCB_SHPR3 = (SCB_SHPR3 & 0x00FFFFFFu) | (0x80u << 24); // SysTick below the UART
  SYST_RVR = SYSCLK_HZ / 1000 - 1;
  SYST_CVR = 0;
  SYST_CSR = 7; // processor clock, interrupt, enable
  uart_init();
}

void board_led(bool on) {
  GPIO_BSRR(GPIOB_BASE) = on ? (1u << 13) : (1u << 29);
}

void board_led_toggle(void) {
  GPIO_ODR(GPIOB_BASE) ^= 1u << 13;
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

void USART2_IRQHandler(void) {
  uint32_t isr = USART2_ISR;
  bool err = (isr & (USART_ISR_ORE | USART_ISR_FE | USART_ISR_NE | USART_ISR_PE)) != 0;
  if (err) USART2_ICR = USART_ISR_ORE | USART_ISR_FE | USART_ISR_NE | USART_ISR_PE;
  if (isr & USART_ISR_RXNE) {
    log_uart_rx((uint8_t)USART2_RDR, err);
  } else if (err) {
    log_uart_rx(0, true); // overrun with no data: a 00 resynchronises the frame scanner
  }
  if ((USART2_CR1 & USART_CR1_TXEIE) && (isr & USART_ISR_TXE)) {
    int c = log_uart_tx_next();
    if (c < 0) {
      USART2_CR1 &= ~USART_CR1_TXEIE;
    } else {
      USART2_TDR = (uint8_t)c;
    }
  }
}

// ---------------------------------------------------------------------------------------------
// ucLog port interface

uint32_t log_port_lock(void) {
  uint32_t primask;
  __asm volatile("mrs %0, primask\n cpsid i" : "=r"(primask)::"memory");
  return primask;
}

void log_port_unlock(uint32_t key) {
  __asm volatile("msr primask, %0" ::"r"(key) : "memory");
}

void log_port_tx_start(void) {
  USART2_CR1 |= USART_CR1_TXEIE; // single read-modify-write; the ISR only clears it
}

void log_port_tx_poll(uint8_t b) {
  while ((USART2_ISR & USART_ISR_TXE) == 0) {
  }
  USART2_TDR = b;
}

uint32_t log_port_ms(void) {
  return ms;
}

__attribute__((noreturn)) void log_port_fatal(void) {
  // Let the last byte leave the shift register.
  for (volatile uint32_t i = 0; i < 100000; i++) {
  }
  if (DHCSR & DHCSR_C_DEBUGEN) __asm volatile("bkpt #0");
  SCB_AIRCR = 0x05FA0004u; // SYSRESETREQ
  while (1) {
  }
}

// ---------------------------------------------------------------------------------------------
// newlib: heap, and the signal stubs abort() needs

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

int _getpid(void) {
  return 1;
}

int _kill(int pid, int sig) {
  errno = EINVAL;
  return -1;
}
