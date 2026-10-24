// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// STM32C071 vector table and reset handler (the STM32CubeC0 startup file, in C). Handlers not
// defined elsewhere go to Default_Handler.

#include <stdint.h>

#include <stm32c0xx.h>

extern uint32_t _estack, _sidata, _sdata, _edata, _sbss, _ebss;
extern int main(void);
extern void __libc_init_array(void);

void Reset_Handler(void);

static void Default_Handler(void) {
  while (1) {
  }
}

#define WEAK __attribute__((weak, alias("Default_Handler")))
void NMI_Handler(void) WEAK;
void HardFault_Handler(void) WEAK;
void SVC_Handler(void) WEAK;
void PendSV_Handler(void) WEAK;
void SysTick_Handler(void) WEAK;
void WWDG_IRQHandler(void) WEAK;
void PVM_VDDIO2_IRQHandler(void) WEAK;
void RTC_IRQHandler(void) WEAK;
void FLASH_IRQHandler(void) WEAK;
void RCC_CRS_IRQHandler(void) WEAK;
void EXTI0_1_IRQHandler(void) WEAK;
void EXTI2_3_IRQHandler(void) WEAK;
void EXTI4_15_IRQHandler(void) WEAK;
void USB_DRD_FS_IRQHandler(void) WEAK;
void DMA1_Channel1_IRQHandler(void) WEAK;
void DMA1_Channel2_3_IRQHandler(void) WEAK;
void DMAMUX1_DMA1_CH4_5_IRQHandler(void) WEAK;
void ADC1_IRQHandler(void) WEAK;
void TIM1_BRK_UP_TRG_COM_IRQHandler(void) WEAK;
void TIM1_CC_IRQHandler(void) WEAK;
void TIM2_IRQHandler(void) WEAK;
void TIM3_IRQHandler(void) WEAK;
void TIM14_IRQHandler(void) WEAK;
void TIM16_IRQHandler(void) WEAK;
void TIM17_IRQHandler(void) WEAK;
void I2C1_IRQHandler(void) WEAK;
void I2C2_IRQHandler(void) WEAK;
void SPI1_IRQHandler(void) WEAK;
void SPI2_IRQHandler(void) WEAK;
void USART1_IRQHandler(void) WEAK;
void USART2_IRQHandler(void) WEAK;

typedef void (*handler_t)(void);

__attribute__((used, section(".isr_vector"))) static const handler_t vectors[16 + 32] = {
    (handler_t)&_estack,
    Reset_Handler,
    NMI_Handler,
    HardFault_Handler,
    [11] = SVC_Handler,
    [14] = PendSV_Handler,
    [15] = SysTick_Handler,
    [16 + WWDG_IRQn] = WWDG_IRQHandler,
    [16 + PVM_VDDIO2_IRQn] = PVM_VDDIO2_IRQHandler,
    [16 + RTC_IRQn] = RTC_IRQHandler,
    [16 + FLASH_IRQn] = FLASH_IRQHandler,
    [16 + RCC_CRS_IRQn] = RCC_CRS_IRQHandler,
    [16 + EXTI0_1_IRQn] = EXTI0_1_IRQHandler,
    [16 + EXTI2_3_IRQn] = EXTI2_3_IRQHandler,
    [16 + EXTI4_15_IRQn] = EXTI4_15_IRQHandler,
    [16 + USB_DRD_FS_IRQn] = USB_DRD_FS_IRQHandler,
    [16 + DMA1_Channel1_IRQn] = DMA1_Channel1_IRQHandler,
    [16 + DMA1_Channel2_3_IRQn] = DMA1_Channel2_3_IRQHandler,
    [16 + DMAMUX1_DMA1_CH4_5_IRQn] = DMAMUX1_DMA1_CH4_5_IRQHandler,
    [16 + ADC1_IRQn] = ADC1_IRQHandler,
    [16 + TIM1_BRK_UP_TRG_COM_IRQn] = TIM1_BRK_UP_TRG_COM_IRQHandler,
    [16 + TIM1_CC_IRQn] = TIM1_CC_IRQHandler,
    [16 + TIM2_IRQn] = TIM2_IRQHandler,
    [16 + TIM3_IRQn] = TIM3_IRQHandler,
    [16 + TIM14_IRQn] = TIM14_IRQHandler,
    [16 + TIM16_IRQn] = TIM16_IRQHandler,
    [16 + TIM17_IRQn] = TIM17_IRQHandler,
    [16 + I2C1_IRQn] = I2C1_IRQHandler,
    [16 + I2C2_IRQn] = I2C2_IRQHandler,
    [16 + SPI1_IRQn] = SPI1_IRQHandler,
    [16 + SPI2_IRQn] = SPI2_IRQHandler,
    [16 + USART1_IRQn] = USART1_IRQHandler,
    [16 + USART2_IRQn] = USART2_IRQHandler,
};

void Reset_Handler(void) {
  uint32_t *s = &_sidata, *d = &_sdata;
  while (d < &_edata) *d++ = *s++;
  for (d = &_sbss; d < &_ebss;) *d++ = 0;
  SystemInit();
  __libc_init_array();
  main();
  while (1) {
  }
}
