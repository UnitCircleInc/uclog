# © 2025 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Zephyr module (zephyr/module.yml): the ucLog device library (src/), its Zephyr port and the
# ucuart/ucusb drivers.

set(UCLOG_ROOT_DIR "${CMAKE_CURRENT_SOURCE_DIR}")

zephyr_include_directories(include)
# Logs name this library's files uclog/... (as on bare metal): no absolute paths, which could contain
# ':' on Windows (log.h).
zephyr_compile_options(-fmacro-prefix-map=${UCLOG_ROOT_DIR}=uclog)
add_subdirectory(drivers)
add_subdirectory(ports/zephyr)
