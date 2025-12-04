
// There seems to be a choice here
// If on Zephyr can use their PM API and have logging framework
// deal calling
// #if CONFIG_DEVICE_RUNTIME
//   pm_device_runtime_get => PM_DEVICE_ACTION_RESUME => rx_start
//   pm_device_runtime_put => PM_DEVICE_ACTION_SUSPEND => rx_stop
// #else
//   pm_device_action_run(PM_DEVICE_ACTION_RESUME) => rx_start
//   pm_device_action_run(PM_DEVICE_ACTION_SUSPEND) => rx_stop
// #endif
//
// Or logging directly call rx_start/rx_stop
// ucuart is mostly generic (for other platforms).
// On zephyr the only likely client is logging framework
// Trying to keep logging framework platform agnostic although could make it
// zephyr compatible with config macros
//
// In general the zephyr API is intended to have the driver itself call
//  the PM actions - but for turning rx on/off this would possibly drop
//  characters - so users need to specifically call the actions rather than
//  the driver infering from api calls.  Note works better for Tx, but not
//  so well for Rx.
//
// Not seeing a lot of advantages for use of PM - not trying to do
// generic uart and generic clients this is logging<->uart tightly coupled.
//
// Need to figure out starting state.
//
// PM_DEVICE_ACTION_TURN_ON/PM_DEVICE_ACTION_TURN_OFF not used/needed
// as uart driver does not use a power domain.
// I guess there maybe cases for a level shifter that might be on a power
// domain - but for logging that shifter is always on external power not
// device power - so no need.

#ifdef CONFIG_PM_DEVICE

static int uart_pm_action(const struct device *dev, enum pm_device_action action) {
  int ret;
  const struct ucuart_config * config = ZEPHYR_DEVICE_MEMBER(dev, config);
  LOG_INF("uart_pc_action: %s", pm_device_state_str(action));
  switch (action) {
    case PM_DEVICE_ACTION_RESUME:
      ret = my_pinctrl_apply_state(config->pcfg, PINCTRL_STATE_DEFAULT);
      if (ret < 0) return ret;
      // Restart rx/tx tasks
      // nrf_uarte_enable(config->regs);
      break;

    case PM_DEVICE_ACTION_SUSPEND:
      // Stop rx/tx tasks - wait for rx/tx stopped to be complete
      // nrf_uarte_disable(config->regs);
      ret = my_pinctrl_apply_state(config->pcfg, PINCTRL_STATE_SLEEP);
      if (ret < 0) return ret;
      break;

    case PM_DEVICE_ACTION_TURN_ON:
    case PM_DEVICE_ACTION_TURN_OFF:
      break;

    default: return -ENOTSUP;
  }
  return 0;
}
#include <zephyr/pm/device.h>

// Notes for logging framework:
// https://docs.zephyrproject.org/latest/services/pm/device.html#device-power-management
// See figure 15.
// Logger should probably be calling:
// pm_device_busy_set()
// pm_device_busy_clear()
// pm_device_wakeup_enable()
// Also see: samples/boards/nrf/system_off/src/main.c

#endif

  PM_DEVICE_DT_INST_DEFINE(i, uart_pm_action);                      \
                                                                    \
  DEVICE_DT_INST_DEFINE(i, ucuart_init, PM_DEVICE_DT_INST_GET(i), &data##i, &config##i, \
      PRE_KERNEL_1, CONFIG_UCUART_INIT_PRIORITY, &ucuart_api);




