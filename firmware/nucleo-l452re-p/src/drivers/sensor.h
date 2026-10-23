// A simulated temperature sensor: logs from a second source file in another directory.
#pragma once

#include <stdint.h>

typedef enum {
  SENSOR_OFF,
  SENSOR_WARMUP,
  SENSOR_READY,
  SENSOR_FAULT = 0x10,
} sensor_state_t;

enum sensor_mode {
  MODE_SLOW = 1,
  MODE_FAST = 2,
  MODE_BURST = 10,
};

void sensor_init(enum sensor_mode mode);
float sensor_read(uint32_t now_ms);
sensor_state_t sensor_state(void);
void sensor_dump(void);
