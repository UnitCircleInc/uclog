// Minimal STM32L452 / Cortex-M4 register definitions (RM0394), only what this board uses.
#pragma once

#include <stdint.h>

#define REG32(a) (*(volatile uint32_t *)(a))

// Cortex-M4 core
#define SYST_CSR REG32(0xE000E010)
#define SYST_RVR REG32(0xE000E014)
#define SYST_CVR REG32(0xE000E018)
#define NVIC_ISER(n) REG32(0xE000E100 + 4 * (n))
#define NVIC_IPR8(n) (*(volatile uint8_t *)(0xE000E400 + (n)))
#define SCB_AIRCR REG32(0xE000ED0C)
#define SCB_SHPR3 REG32(0xE000ED20)
#define SCB_CPACR REG32(0xE000ED88)
#define DHCSR REG32(0xE000EDF0)
#define DHCSR_C_DEBUGEN (1u << 0)

// RCC
#define RCC_BASE 0x40021000u
#define RCC_CR REG32(RCC_BASE + 0x00)
#define RCC_CFGR REG32(RCC_BASE + 0x08)
#define RCC_PLLCFGR REG32(RCC_BASE + 0x0C)
#define RCC_AHB2ENR REG32(RCC_BASE + 0x4C)
#define RCC_APB1ENR1 REG32(RCC_BASE + 0x58)
#define RCC_CR_HSION (1u << 8)
#define RCC_CR_HSIRDY (1u << 10)
#define RCC_CR_PLLON (1u << 24)
#define RCC_CR_PLLRDY (1u << 25)

// FLASH
#define FLASH_ACR REG32(0x40022000u)

// GPIO
#define GPIOA_BASE 0x48000000u
#define GPIOB_BASE 0x48000400u
#define GPIO_MODER(b) REG32((b) + 0x00)
#define GPIO_OSPEEDR(b) REG32((b) + 0x08)
#define GPIO_PUPDR(b) REG32((b) + 0x0C)
#define GPIO_ODR(b) REG32((b) + 0x14)
#define GPIO_BSRR(b) REG32((b) + 0x18)
#define GPIO_AFRL(b) REG32((b) + 0x20)

// USART2
#define USART2_BASE 0x40004400u
#define USART2_CR1 REG32(USART2_BASE + 0x00)
#define USART2_BRR REG32(USART2_BASE + 0x0C)
#define USART2_ISR REG32(USART2_BASE + 0x1C)
#define USART2_ICR REG32(USART2_BASE + 0x20)
#define USART2_RDR REG32(USART2_BASE + 0x24)
#define USART2_TDR REG32(USART2_BASE + 0x28)
#define USART_CR1_UE (1u << 0)
#define USART_CR1_RE (1u << 2)
#define USART_CR1_TE (1u << 3)
#define USART_CR1_RXNEIE (1u << 5)
#define USART_CR1_TXEIE (1u << 7)
#define USART_ISR_PE (1u << 0)
#define USART_ISR_FE (1u << 1)
#define USART_ISR_NE (1u << 2)
#define USART_ISR_ORE (1u << 3)
#define USART_ISR_RXNE (1u << 5)
#define USART_ISR_TXE (1u << 7)
#define USART2_IRQn 38

// 96-bit unique device ID
#define UID_BASE 0x1FFF7590u
