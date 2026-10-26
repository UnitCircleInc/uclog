// © 2025 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Zephyr's LOG_ERR, LOG_WRN, LOG_INF, LOG_DBG and LOG_HEXDUMP_* as ucLog logs, with Zephyr's
// per-module levels (CONFIG_LOG_CUSTOM_HEADER=y includes this from <zephyr/logging/log.h>).

#pragma once

#include "log.h"

#undef LOG_ERR
#undef LOG_WRN
#undef LOG_INF
#undef LOG_DBG
#undef LOG_RAW
#undef LOG_WRN_ONCE
#undef LOG_HEXDUMP_ERR
#undef LOG_HEXDUMP_WRN
#undef LOG_HEXDUMP_INF
#undef LOG_HEXDUMP_DBG

#define UC_ZLOG_(lvl_, log_)                                                                   \
  do {                                                                                         \
    if (Z_LOG_CONST_LEVEL_CHECK(lvl_)) {                                                       \
      log_;                                                                                    \
    }                                                                                          \
  } while (0)

#define LOG_ERR(...) UC_ZLOG_(LOG_LEVEL_ERR, LOG_ERROR(__VA_ARGS__))
#define LOG_WRN(...) UC_ZLOG_(LOG_LEVEL_WRN, LOG_WARN(__VA_ARGS__))
#define LOG_INF(...) UC_ZLOG_(LOG_LEVEL_INF, LOG_INFO(__VA_ARGS__))
#define LOG_DBG(...) UC_ZLOG_(LOG_LEVEL_DBG, LOG_DEBUG(__VA_ARGS__))

#define LOG_WRN_ONCE(...)                                                                      \
  do {                                                                                         \
    static uint8_t warned__ = 0;                                                               \
    if (warned__ == 0) {                                                                       \
      LOG_WRN(__VA_ARGS__);                                                                    \
      warned__ = 1;                                                                            \
    }                                                                                          \
  } while (0)

#define LOG_RAW(...) LOG_ERROR("LOG_RAW - not supported")

#define LOG_HEXDUMP_ERR(data_, length_, str_) UC_ZLOG_(LOG_LEVEL_ERR, LOG_MEM_ERROR(str_, data_, length_))
#define LOG_HEXDUMP_WRN(data_, length_, str_) UC_ZLOG_(LOG_LEVEL_WRN, LOG_MEM_WARN(str_, data_, length_))
#define LOG_HEXDUMP_INF(data_, length_, str_) UC_ZLOG_(LOG_LEVEL_INF, LOG_MEM_INFO(str_, data_, length_))
#define LOG_HEXDUMP_DBG(data_, length_, str_) UC_ZLOG_(LOG_LEVEL_DBG, LOG_MEM_DEBUG(str_, data_, length_))
