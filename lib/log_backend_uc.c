// © 2023 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

#include <zephyr/logging/log_backend.h>
#include <zephyr/logging/log_core.h>
#include <zephyr/logging/log_output.h>
#include <zephyr/logging/log_backend_std.h>
#include <zephyr/logging/log.h>

#include <log.h>

static volatile bool in_panic;
static uint32_t log_format_current = CONFIG_LOG_BACKEND_UC_OUTPUT_DEFAULT;

static uint8_t buffer[100];
static size_t  idx = 0;
static int char_out(uint8_t *data, size_t length, void *ctx) {
  ARG_UNUSED(ctx);
  size_t n = length;
  while (n-- > 0) {
    if (idx >= sizeof(buffer)) {
      // flush what we have. will be truncated and split into two prints
      log_mem_(LOG_STRING_("0:<zephyr trunc>:<zephyr>:<zephyr>"), buffer, idx);
      idx = 0;
    }
    else if (*data == 0x0d) {
      // do nothing
    }
    else if (*data == 0x0a) {
      // This relies on internal implementation details
      // logdata.py has the matching checks
      log_mem_(LOG_STRING_("0:<zephyr>:<zephyr>:<zephyr>"), buffer, idx);
      idx = 0;
    }
    else {
      buffer[idx++] = *data;
    }
    data++;
  }
  return length;
}

static uint8_t uc_output_buf[CONFIG_LOG_BACKEND_UC_BUFFER_SIZE];
LOG_OUTPUT_DEFINE(log_output_uc, char_out, uc_output_buf, sizeof(uc_output_buf));

static void process(const struct log_backend *const backend,
		union log_msg_generic *msg) {
  uint32_t flags = 0 /*log_backend_std_get_flags() |
                   LOG_OUTPUT_FLAG_FORMAT_TIMESTAMP |
                   LOG_OUTPUT_FLAG_COLORS */;

  log_format_func_t log_output_func = log_format_func_t_get(log_format_current);

  log_output_func(&log_output_uc, &msg->log, flags);
}

static int format_set(const struct log_backend *const backend, uint32_t log_type) {
  log_format_current = log_type;
  return 0;
}

static void log_backend_uc_init(struct log_backend const *const backend) {
}

static void panic(struct log_backend const *const backend) {
  in_panic = true;
  log_backend_std_panic(&log_output_uc);
}

static void dropped(const struct log_backend *const backend, uint32_t cnt) {
  ARG_UNUSED(backend);
  log_backend_std_dropped(&log_output_uc, cnt);
}

const struct log_backend_api log_backend_uc_api = {
  .process = process,
  .panic = panic,
  .init = log_backend_uc_init,
  .dropped = IS_ENABLED(CONFIG_LOG_MODE_IMMEDIATE) ? NULL : dropped,
  .format_set = format_set,
};

LOG_BACKEND_DEFINE(log_backend_uc, log_backend_uc_api, true);
