#include "sensor.h"

#include <math.h>
#include <stddef.h>

#include "log.h"

static sensor_state_t state = SENSOR_OFF;
static enum sensor_mode mode;
static float history[8];
static size_t history_n;
static uint32_t reads;

void sensor_init(enum sensor_mode m) {
  mode = m;
  state = SENSOR_WARMUP;
  LOG_INFO("sensor init: mode {enum:sensor_mode}%d state {enum:sensor_state_t}%d", mode, state);
}

static void set_state(sensor_state_t s) {
  if (s != state) {
    LOG_INFO("sensor state {enum:sensor_state_t}%d -> {enum:sensor_state_t}%d", state, s);
    state = s;
  }
}

float sensor_read(uint32_t now_ms) {
  reads++;
  float t = 21.5f + 2.0f * sinf((float)now_ms * 0.0005f);
  history[history_n++ % 8] = t;
  if (state == SENSOR_WARMUP && reads >= 3) set_state(SENSOR_READY);
  if (t > 23.4f) {
    LOG_WARN("sensor: temperature high %.2f C (limit %.1f)", t, 23.4);
  }
  LOG_DEBUG("sensor read #%lu: %f", (unsigned long)reads, t);
  return t;
}

sensor_state_t sensor_state(void) {
  return state;
}

void sensor_dump(void) {
  LOG_MEM_INFO("sensor history (float[8])", history, sizeof(history));
  LOG_INFO("sensor history at {sym}%p, reader at {sym}%p", (void *)&history[2], (void *)sensor_read);
}
