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

#pragma once

#include <stm32l4xx_ll_gpio.h>
#include <stdint.h>
#include <stdbool.h>

typedef struct {
  GPIO_TypeDef* o_port;
  uint32_t      o_pin;
  uint32_t      o_pull;
  uint32_t      o_type;
  uint32_t      o_speed;
  uint32_t      o_init;
} gpio_output_t;

typedef struct {
  GPIO_TypeDef* i_port;
  uint32_t      i_pin;
  uint32_t      i_pull;
} gpio_input_t;

typedef struct {
  GPIO_TypeDef* a_port;
  uint32_t      a_pin;
} gpio_analog_t;


typedef struct {
  GPIO_TypeDef* af_port;
  uint32_t      af_pin;
  uint32_t      af_pull;
  uint32_t      af_type;
  uint32_t      af_speed;
  uint32_t      af_af;
} gpio_alternate_t;


#define GPIO_OUTPUT(port, pin, pull, type, speed, init) {\
  .o_port = GPIO##port, \
  .o_pin  = LL_GPIO_PIN_##pin, \
  .o_pull = LL_GPIO_PULL_##pull, \
  .o_type = LL_GPIO_OUTPUT_##type, \
  .o_speed = LL_GPIO_SPEED_FREQ_##speed, \
  .o_init = init \
}

#define GPIO_ANALOG(port, pin) { \
  .a_port = GPIO##port, \
  .a_pin  = LL_GPIO_PIN_##pin, \
}

#define GPIO_INPUT(port, pin, pull) { \
  .i_port = GPIO##port, \
  .i_pin  = LL_GPIO_PIN_##pin, \
  .i_pull = LL_GPIO_PULL_##pull \
}

#define GPIO_ALTERNATE(port, pin, pull, type, speed, af) { \
  .af_port = GPIO##port, \
  .af_pin = LL_GPIO_PIN_##pin, \
  .af_pull = LL_GPIO_PULL_##pull, \
  .af_type = LL_GPIO_OUTPUT_##type, \
  .af_speed = LL_GPIO_SPEED_FREQ_##speed, \
  .af_af = af \
}

void gpio_output(const gpio_output_t* config);
void gpio_input(const gpio_input_t* config);
void gpio_analog(const gpio_analog_t* config);
void gpio_alternate(const gpio_alternate_t* config);
void gpio_out(const gpio_output_t* config, uint32_t v);
void gpio_toggle(const gpio_output_t* config);
bool gpio_in(const gpio_input_t* config);
void gpio_enable_clk(const void* config);
void gpio_disable_clk(const void* config);
