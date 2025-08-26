// © 2022 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

#include "ucuart.h"
#include "cb.h"
#include <hal/nrf_uarte.h>
#include <hal/nrf_gpio.h>
#include <nrfx_timer.h>
#include <nrfx_ppi.h>

// This must be a power of 2 in order to be able to use 32 bit timer/counter
// mod and not have to keep track of timer/counter overflow events.
#define RX_BUF_LEN (2<<7)

// TODO move to another file e.g. ucpinctrl.h
#ifdef CONFIG_PINCTRL
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

static inline int my_pinctrl_apply_state(
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


#else
#error CONFIG_PINCTRL required.
#endif


// Must match dts/bindings file - commas replaced with underscores
#define DT_DRV_COMPAT unitcircle_ucuart

struct ucuart_config {
  NRF_UARTE_Type * regs;
  uint32_t current_speed;
  const struct pinctrl_dev_config *pcfg;
  void (*irq_config)(const struct device *dev);
  cb_t* rx_cb;

  nrfx_timer_t      timer;
};

struct ucuart_data {
  atomic_t tx_active;
  atomic_t rx_active;
  cb_t* tx_cb;
  uint32_t last_error;

  struct k_event event;
  size_t n; // current number of bytes being sent
  nrf_ppi_channel_t ppi;
};

typedef enum {
  UCUART_ERROR_NONE = 0,
  UCUART_ERROR_TX_DMA = 1 << 0,
  UCUART_ERROR_RX_DMA = 1 << 1,
  UCUART_ERROR_OR     = 1 << 2,
  UCUART_ERROR_FRAMING= 1 << 3,
  UCUART_ERROR_NOISE  = 1 << 4,
} uart_error_t;

static void uart_handler(const struct device* dev) {
  const struct ucuart_config * config = ZEPHYR_DEVICE_MEMBER(dev, config);
  struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);

  if (nrf_uarte_event_check(config->regs, NRF_UARTE_EVENT_ERROR)) {
    data->last_error = nrf_uarte_errorsrc_get_and_clear(config->regs);
    nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_ERROR);
  }
  if (nrf_uarte_event_check(config->regs, NRF_UARTE_EVENT_ENDRX)) {
    // This will trigger a STARTRX
    nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_ENDRX);
  }
  if (nrf_uarte_event_check(config->regs, NRF_UARTE_EVENT_RXDRDY)) {
    nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_RXDRDY);
    atomic_cas(&data->rx_active, false, true);
    k_event_post(&data->event, UCUART_EVT_RX);
  }

  if (nrf_uarte_event_check(config->regs, NRF_UARTE_EVENT_RXTO)) {
    nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_RXTO);
  }

  if (nrf_uarte_event_check(config->regs, NRF_UARTE_EVENT_RXSTARTED)) {
    // Setup for next buffer after current RX completes
    nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_RXSTARTED);
    nrf_uarte_rx_buffer_set(config->regs, config->rx_cb->b, RX_BUF_LEN);
  }


  if (nrf_uarte_event_check(config->regs, NRF_UARTE_EVENT_ENDTX)) {
    nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_ENDTX);
    if (atomic_get(&data->tx_active)) {
      nrf_uarte_task_trigger(config->regs, NRF_UARTE_TASK_STOPTX);
    }
  }

  if (nrf_uarte_event_check(config->regs, NRF_UARTE_EVENT_TXSTOPPED)) {
    nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_TXSTOPPED);
    cb_skip(data->tx_cb, data->n);

    // If there is more data then send it now
    size_t n = cb_peek_avail(data->tx_cb);
    if (n > 0) {
      nrf_uarte_tx_buffer_set(config->regs, cb_peek(data->tx_cb), n);
      data->n = n;
      nrf_uarte_task_trigger(config->regs, NRF_UARTE_TASK_STARTTX);
    }
    else {
      atomic_set(&data->tx_active, false);
      data->n = 0U;
    }
  }
}


static inline uint32_t br2uartebr(uint32_t br) {
  switch (br) {
    case     300: return 0x00014000;
    case     600: return 0x00027000;
    case    1200: return NRF_UARTE_BAUDRATE_1200;
    case    2400: return NRF_UARTE_BAUDRATE_2400;
    case    4800: return NRF_UARTE_BAUDRATE_4800;
    case    9600: return NRF_UARTE_BAUDRATE_9600;
    case   14400: return NRF_UARTE_BAUDRATE_14400;
    case   19200: return NRF_UARTE_BAUDRATE_19200;
    case   28800: return NRF_UARTE_BAUDRATE_28800;
    case   31250: return NRF_UARTE_BAUDRATE_31250;
    case   38400: return NRF_UARTE_BAUDRATE_38400;
    case   56000: return NRF_UARTE_BAUDRATE_56000;
    case   57600: return NRF_UARTE_BAUDRATE_57600;
    case   76800: return NRF_UARTE_BAUDRATE_76800;
    case  115200: return NRF_UARTE_BAUDRATE_115200;
    case  230400: return NRF_UARTE_BAUDRATE_230400;
    case  250000: return NRF_UARTE_BAUDRATE_250000;
    case  460800: return NRF_UARTE_BAUDRATE_460800;
    case  921600: return NRF_UARTE_BAUDRATE_921600;
    case 1000000: return NRF_UARTE_BAUDRATE_1000000;
    default:      return 0; // 0 indicates error
  }
}

static int tx_schedule(const struct device *dev) {
  const struct ucuart_config * config = ZEPHYR_DEVICE_MEMBER(dev, config);
  struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);

  if (data->tx_cb) {
    bool got = atomic_cas(&data->tx_active, false, true);
    if (got) {
      size_t n = cb_peek_avail(data->tx_cb);
      if (n > 0) {
        nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_ENDTX);
        nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_TXSTOPPED);
        nrf_uarte_tx_buffer_set(config->regs, cb_peek(data->tx_cb), n);
        data->n = n;
        nrf_uarte_task_trigger(config->regs, NRF_UARTE_TASK_STARTTX);
      }
      else {
        atomic_set(&data->tx_active, false);
      }
    }
  }
  return 0;
}

static int tx(const struct device *dev, const uint8_t* b, size_t n) {
  const struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);

  if (data->tx_cb == NULL) return -EIO;

  cb_write(data->tx_cb, b, n);
  return tx_schedule(dev);
}

static int tx_buffer(const struct device *dev, const uint8_t* b, size_t n) {
  const struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);

  if (data->tx_cb == NULL) return -EIO;

  cb_write(data->tx_cb, b, n);
  return 0;
}

static int set_tx_cb(const struct device *dev, cb_t* cb) {
  struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);
  data->tx_cb = cb;
  return 0;
}

#if 0
// TODO Add to API perhaps should be an error or a function to convert
// error code to error string for those that want it.
//
static uint32_t last_error(const struct device *dev) {
  const struct ucuart_config * config = ZEPHYR_DEVICE_MEMBER(dev, config);
  uint32_t last_error = dev->last_error;
  if (last_error != 0) {
    LOG_ERROR("uart error: %c%c%c%c",
       (last_error & NRF_UARTE_ERROR_OVERRUN_MASK) == 0 ? '-' : 'O',
       (last_error & NRF_UARTE_ERROR_PARITY_MASK)  == 0 ? '-' : 'P',
       (last_error & NRF_UARTE_ERROR_FRAMING_MASK) == 0 ? '-' : 'F',
       (last_error & NRF_UARTE_ERROR_BREAK_MASK)   == 0 ? '-' : 'B');
  }
  dev->last_error = 0;
  return last_error;
}
#endif


static void rx_start(const struct device *dev) {
  struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);
  (void) data;
}

static void rx_stop(const struct device *dev) {
  struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);
  (void) data;
}

static size_t rx_avail(const struct device *dev) {
  const struct ucuart_config * config = ZEPHYR_DEVICE_MEMBER(dev, config);
  return cb_peek_avail(config->rx_cb);
}

static const uint8_t* rx_peek(const struct device *dev) {
  const struct ucuart_config * config = ZEPHYR_DEVICE_MEMBER(dev, config);
  return cb_peek(config->rx_cb);
}

static void rx_skip(const struct device *dev, size_t n) {
  const struct ucuart_config * config = ZEPHYR_DEVICE_MEMBER(dev, config);
  cb_skip(config->rx_cb, n);
}


static uint32_t wait_event(const struct device *dev, uint32_t mask, bool reset, k_timeout_t timeout) {
  // It is OK for events to occur between k_event_wait and k_event_clear.
  // The data associated with these events will be picked up with the capture.
  // The client must ensure that before calling wait_event the call to
  // uart_rx_avail returns 0;
  struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);
  const struct ucuart_config * config = ZEPHYR_DEVICE_MEMBER(dev, config);
  (void) reset;
  uint32_t r =  k_event_wait(&data->event, mask, 0, timeout);
  if (r != 0) k_event_clear(&data->event, r);
  size_t w = nrfx_timer_capture(&config->timer, 0) % RX_BUF_LEN;
  config->rx_cb->write = w;
  return r;
}

static int panic(const struct device *dev) {
  (void) dev;
  // TODO Need to call interrupt handler like USB driver does
  return 0;
}

// Perhaps rx_start/rx_stop should be run/idle
// with run having tx/rx enabled at full power, and idle
// having peripheral disabled, and Rx pin as GPIO with interrupt
// semantics.  Idle -> lower power, less functionality.
// resume/suspend - although suspend is not realy full suspend.
//
static const struct ucuart_driver_api ucuart_api = {
  .tx_no_wait = tx,
  .tx_buffer = tx_buffer,
  .tx_schedule = tx_schedule,
  .set_tx_cb = set_tx_cb,
  .rx_start = rx_start,
  .rx_stop = rx_stop,
  .rx_avail = rx_avail,
  .rx_peek = rx_peek,
  .rx_skip = rx_skip,
  .wait_event = wait_event,
  .panic = panic,
};

static void timer_handler(nrf_timer_event_t event_type, void *p_context) { }

static int ucuart_init(const struct device *dev) {
  const struct ucuart_config * config = ZEPHYR_DEVICE_MEMBER(dev, config);
  struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);
  int err;

  k_event_init(&data->event);

  nrf_uarte_disable(config->regs);

  err = my_pinctrl_apply_state(config->pcfg, PINCTRL_STATE_DEFAULT);
  if (err < 0) return err;

  nrf_uarte_baudrate_set(config->regs, br2uartebr(config->current_speed));

  nrf_uarte_config_t uarte_cfg = {
    .hwfc = NRF_UARTE_HWFC_DISABLED,
    .parity = NRF_UARTE_PARITY_EXCLUDED,
#if defined(UARTE_CONFIG_STOP_Msk)
    .stop = NRF_UARTE_STOP_ONE,
#endif
  };
  nrf_uarte_configure(config->regs, &uarte_cfg);

  // Enable interrupts
  nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_ENDRX);
  nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_ERROR);
  nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_RXTO);
  nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_RXDRDY);
  nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_ENDTX);
  nrf_uarte_event_clear(config->regs, NRF_UARTE_EVENT_TXSTOPPED);
  nrf_uarte_int_enable(config->regs, NRF_UARTE_INT_ENDRX_MASK   |
                                     NRF_UARTE_INT_ERROR_MASK   |
                                     NRF_UARTE_INT_RXTO_MASK    |
                                     NRF_UARTE_INT_RXDRDY_MASK  |
                                     NRF_UARTE_INT_ENDTX_MASK   |
                                     NRF_UARTE_INT_TXSTOPPED_MASK);

  config->irq_config(dev);

  nrfx_timer_config_t tmr_config = NRFX_TIMER_DEFAULT_CONFIG(
                NRF_TIMER_BASE_FREQUENCY_GET(config->timer.p_reg));

  tmr_config.mode = NRF_TIMER_MODE_COUNTER;
  tmr_config.bit_width = NRF_TIMER_BIT_WIDTH_32;
  err = nrfx_timer_init(&config->timer, &tmr_config, timer_handler);
  if (err != NRFX_SUCCESS) {
    return -EIO;
  }
  nrfx_timer_enable(&config->timer);
  nrfx_timer_clear(&config->timer);

  err = nrfx_ppi_channel_alloc(&data->ppi);
  if (err != NRFX_SUCCESS) {
    return -EIO;
  }

  err = nrfx_ppi_channel_assign(data->ppi,
            nrf_uarte_event_address_get(config->regs, NRF_UARTE_EVENT_RXDRDY),
            nrfx_timer_task_address_get(&config->timer, NRF_TIMER_TASK_COUNT));
  if (err != NRFX_SUCCESS) {
    return -EIO;
  }
  err = nrfx_ppi_channel_enable(data->ppi);
  if (err != NRFX_SUCCESS) {
    return -EIO;
  }

  nrf_uarte_enable(config->regs);

  nrf_uarte_shorts_enable(config->regs, NRF_UARTE_SHORT_ENDRX_STARTRX);
  nrf_uarte_rx_buffer_set(config->regs, config->rx_cb->b, RX_BUF_LEN);
  nrf_uarte_task_trigger(config->regs, NRF_UARTE_TASK_STARTRX);
  return 0;
}

// TODO Timer should be configurable from device tree.
#define CONFIG_UCUART_0_TIMER 4

#define IRQ_CONFIG(i)                                               \
static void irq_config##i(const struct device *dev) {               \
  IRQ_CONNECT(DT_INST_IRQN(i),                                      \
              DT_INST_IRQ(i, priority),                             \
              uart_handler, DEVICE_DT_INST_GET(i), 0);              \
  irq_enable(DT_INST_IRQN(i));           \
}

#define UCUART_DEFINE(i)                                            \
  IRQ_CONFIG(i)                                                     \
  PINCTRL_DT_INST_DEFINE(i);                                        \
                                                                    \
                                                                    \
  static uint8_t ucuart_rx_buf##i[RX_BUF_LEN];                      \
  static cb_t ucuart_rx_cb##i = CB_INIT(ucuart_rx_buf##i);          \
                                                                    \
  static const struct ucuart_config config##i = {                   \
    .regs = (NRF_UARTE_Type *) DT_INST_REG_ADDR(i),                 \
    .current_speed = DT_INST_PROP(i, current_speed),                \
    .pcfg = PINCTRL_DT_INST_DEV_CONFIG_GET(i),                      \
    .irq_config = irq_config##i,                                    \
    .rx_cb = &ucuart_rx_cb##i,                                      \
    .timer = NRFX_TIMER_INSTANCE(CONFIG_UCUART_##i##_TIMER),\
  };                                                                \
                                                                    \
  static struct ucuart_data data##i = {                             \
    .tx_active = false,                                             \
    .rx_active = false,                                             \
    .tx_cb = NULL,                                                  \
    .last_error = 0,                                                \
    .n = 0U,                                                        \
  };                                                                \
                                                                    \
  DEVICE_DT_INST_DEFINE(i, ucuart_init, PM_DEVICE_DT_INST_GET(i), &data##i, &config##i, \
      PRE_KERNEL_1, CONFIG_UCUART_INIT_PRIORITY, &ucuart_api);

DT_INST_FOREACH_STATUS_OKAY(UCUART_DEFINE)
