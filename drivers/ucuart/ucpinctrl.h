// © 2025 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
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

#ifndef CONFIG_PINCTRL
#error CONFIG_PINCTRL required.
#endif

#include <zephyr/drivers/pinctrl.h>

// The following is a specialized version of drivers/pinctrl/pinctrl_nrf.h
// that just handles the UART realted pin control functions.
// And this seems easier than replacing pinctrl data structures which would
// require re-jigging the device tree side of things.
// Main down side is kepping synchronized with Zephyr if they make significant
// changes to implementation of pinctrl_apply_state.
BUILD_ASSERT(((NRF_PULL_NONE == NRF_GPIO_PIN_NOPULL) &&
	      (NRF_PULL_DOWN == NRF_GPIO_PIN_PULLDOWN) &&
	      (NRF_PULL_UP == NRF_GPIO_PIN_PULLUP)),
	      "nRF pinctrl pull settings do not match HAL values");

#if defined(GPIO_PIN_CNF_DRIVE_E0E1) || defined(GPIO_PIN_CNF_DRIVE0_E0)
#define NRF_DRIVE_COUNT (NRF_DRIVE_E0E1 + 1)
#else
#define NRF_DRIVE_COUNT (NRF_DRIVE_H0D1 + 1)
#endif
static const nrf_gpio_pin_drive_t drive_modes[NRF_DRIVE_COUNT] = {
	[NRF_DRIVE_S0S1] = NRF_GPIO_PIN_S0S1,
	[NRF_DRIVE_H0S1] = NRF_GPIO_PIN_H0S1,
	[NRF_DRIVE_S0H1] = NRF_GPIO_PIN_S0H1,
	[NRF_DRIVE_H0H1] = NRF_GPIO_PIN_H0H1,
	[NRF_DRIVE_D0S1] = NRF_GPIO_PIN_D0S1,
	[NRF_DRIVE_D0H1] = NRF_GPIO_PIN_D0H1,
	[NRF_DRIVE_S0D1] = NRF_GPIO_PIN_S0D1,
	[NRF_DRIVE_H0D1] = NRF_GPIO_PIN_H0D1,
#if defined(GPIO_PIN_CNF_DRIVE_E0E1) || defined(GPIO_PIN_CNF_DRIVE0_E0)
	[NRF_DRIVE_E0E1] = NRF_GPIO_PIN_E0E1,
#endif
};

#define NO_WRITE UINT32_MAX

#define PSEL_DISCONNECTED 0xFFFFFFFFUL

#define NRF_PSEL_UART(reg, line) ((NRF_UARTE_Type *)reg)->PSEL.line

static inline int uc_pinctrl_apply_state(
    const struct pinctrl_dev_config *config, uint8_t id) {
  int ret;
  const struct pinctrl_state *state;

  ret = pinctrl_lookup_state(config, id, &state);
  if (ret < 0) {
    return ret;
  }
  uintptr_t reg = config->reg;
  const pinctrl_soc_pin_t *pins = state->pins;
  uint8_t pin_cnt = state->pin_cnt;

  for (uint8_t i = 0U; i < pin_cnt; i++) {
    nrf_gpio_pin_drive_t drive;
    uint8_t drive_idx = NRF_GET_DRIVE(pins[i]);
    uint32_t psel = NRF_GET_PIN(pins[i]);
    uint32_t write = NO_WRITE;
    nrf_gpio_pin_dir_t dir;
    nrf_gpio_pin_input_t input;

    if (drive_idx < ARRAY_SIZE(drive_modes)) {
      drive = drive_modes[drive_idx];
    } else {
      return -EINVAL;
    }

    if (psel == NRF_PIN_DISCONNECTED) {
      psel = PSEL_DISCONNECTED;
    }

    switch (NRF_GET_FUN(pins[i])) {
    case NRF_FUN_UART_TX:
      NRF_PSEL_UART(reg, TXD) = psel;
      write = 1U;
      dir = NRF_GPIO_PIN_DIR_OUTPUT;
      input = NRF_GPIO_PIN_INPUT_DISCONNECT;
      break;
    case NRF_FUN_UART_RX:
      NRF_PSEL_UART(reg, RXD) = psel;
      dir = NRF_GPIO_PIN_DIR_INPUT;
      input = NRF_GPIO_PIN_INPUT_CONNECT;
      break;
    case NRF_FUN_UART_RTS:
      NRF_PSEL_UART(reg, RTS) = psel;
      write = 1U;
      dir = NRF_GPIO_PIN_DIR_OUTPUT;
      input = NRF_GPIO_PIN_INPUT_DISCONNECT;
      break;
    case NRF_FUN_UART_CTS:
      NRF_PSEL_UART(reg, CTS) = psel;
      dir = NRF_GPIO_PIN_DIR_INPUT;
      input = NRF_GPIO_PIN_INPUT_CONNECT;
      break;
    default:
      return -ENOTSUP;
    }

    /* configure GPIO properties */
    if (psel != PSEL_DISCONNECTED) {
      uint32_t pin = psel;

      if (write != NO_WRITE) {
        nrf_gpio_pin_write(pin, write);
      }

      /* force input and disconnected buffer for low power */
      if (NRF_GET_LP(pins[i]) == NRF_LP_ENABLE) {
        dir = NRF_GPIO_PIN_DIR_INPUT;
        input = NRF_GPIO_PIN_INPUT_DISCONNECT;
      }

      nrf_gpio_cfg(pin, dir, input, NRF_GET_PULL(pins[i]),
             drive, NRF_GPIO_PIN_NOSENSE);
    }
  }

  return 0;
}


