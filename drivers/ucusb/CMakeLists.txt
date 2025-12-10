# © 2022 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

zephyr_library()
zephyr_library_sources_ifdef(CONFIG_UCUSB ucusb.c)
