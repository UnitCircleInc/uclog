# © 2025 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

zephyr_library()

#zephyr_syscall_header_ifdef(CONFIG_UCUART ${APPLICATION_SOURCE_DIR}/../include/ucuart.h)
zephyr_syscall_header(${APPLICATION_SOURCE_DIR}/../include/ucuart.h)

zephyr_library_sources_ifdef(CONFIG_UCUART ucuart.c)
