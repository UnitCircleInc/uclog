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

#include <gpio.h>
#include <stm32l4xx_ll_gpio.h>
#include <stm32l4xx_ll_bus.h>

void gpio_output(const gpio_output_t* config) {
  LL_GPIO_SetPinPull(config->o_port, config->o_pin, config->o_pull);
  LL_GPIO_SetPinOutputType(config->o_port, config->o_pin, config->o_type);
  LL_GPIO_SetPinSpeed(config->o_port, config->o_pin, config->o_speed);
  if (config->o_init) {
    LL_GPIO_SetOutputPin(config->o_port, config->o_pin);
  }
  else {
    LL_GPIO_ResetOutputPin(config->o_port, config->o_pin);
  }
  LL_GPIO_SetPinMode(config->o_port, config->o_pin, LL_GPIO_MODE_OUTPUT);
}

void gpio_input(const gpio_input_t* config) {
  LL_GPIO_SetPinPull(config->i_port, config->i_pin, config->i_pull);
  LL_GPIO_SetPinMode(config->i_port, config->i_pin, LL_GPIO_MODE_INPUT);
}

void gpio_analog(const gpio_analog_t* config) {
  LL_GPIO_SetPinMode(config->a_port, config->a_pin, LL_GPIO_MODE_ANALOG);
}

void gpio_alternate(const gpio_alternate_t* config) {
  LL_GPIO_SetPinPull(config->af_port, config->af_pin, config->af_pull);
  LL_GPIO_SetPinOutputType(config->af_port, config->af_pin, config->af_type);
  LL_GPIO_SetPinSpeed(config->af_port, config->af_pin, config->af_speed);
  if (config->af_pin < LL_GPIO_PIN_8) {
    LL_GPIO_SetAFPin_0_7(config->af_port, config->af_pin, config->af_af);
  }
  else {
    LL_GPIO_SetAFPin_8_15(config->af_port, config->af_pin, config->af_af);
  }
  LL_GPIO_SetPinMode(config->af_port, config->af_pin, LL_GPIO_MODE_ALTERNATE);
}

void gpio_out(const gpio_output_t* config, uint32_t v) {
  if (v) {
    LL_GPIO_SetOutputPin(config->o_port, config->o_pin);
  }
  else {
    LL_GPIO_ResetOutputPin(config->o_port, config->o_pin);
  }
}

void gpio_toggle(const gpio_output_t* config) {
  LL_GPIO_TogglePin(config->o_port, config->o_pin);
}

bool gpio_in(const gpio_input_t* config) {
  return LL_GPIO_IsInputPinSet(config->i_port, config->i_pin);
}

static uint32_t gpio_port_to_peripheral(const GPIO_TypeDef* port) {
    switch ((uint32_t) port) {
#if defined(GPIOA)
    case GPIOA_BASE: return LL_AHB2_GRP1_PERIPH_GPIOA;
#endif
#if defined(GPIOB)
    case GPIOB_BASE: return LL_AHB2_GRP1_PERIPH_GPIOB;
#endif
#if defined(GPIOC)
    case GPIOC_BASE: return LL_AHB2_GRP1_PERIPH_GPIOC;
#endif
#if defined(GPIOD)
    case GPIOD_BASE: return LL_AHB2_GRP1_PERIPH_GPIOD;
#endif
#if defined(GPIOE)
    case GPIOE_BASE: return LL_AHB2_GRP1_PERIPH_GPIOE;
#endif
#if defined(GPIOF)
    case GPIOF_BASE: return LL_AHB2_GRP1_PERIPH_GPIOF;
#endif
#if defined(GPIOG)
    case GPIOG_BASE: return LL_AHB2_GRP1_PERIPH_GPIOG;
#endif
#if defined(GPIOH)
    case GPIOH_BASE: return LL_AHB2_GRP1_PERIPH_GPIOH;
#endif
#if defined(GPIOI)
    case GPIOI_BASE: return LL_AHB2_GRP1_PERIPH_GPIOI;
#endif
    default: return 0;
  }
}

void gpio_enable_clk(const void* config) {
  const gpio_analog_t* c = config;
  LL_AHB2_GRP1_EnableClock(gpio_port_to_peripheral(c->a_port));
}

void gpio_disable_clk(const void* config) {
  const gpio_analog_t* c = config;
  LL_AHB2_GRP1_DisableClock(gpio_port_to_peripheral(c->a_port));
}


