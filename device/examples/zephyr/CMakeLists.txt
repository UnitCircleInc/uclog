# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
#
# A minimal ucLog app on Zephyr. The board's overlay (boards/) picks the link; the uclog-target0
# snippet adds the LOGDATA region the format strings go in (and ucLog's Kconfig defaults):
#
#   west build -b nucleo_l452re/stm32l452xx/p path/to/examples/zephyr

cmake_minimum_required(VERSION 3.20.0)

# `west build -S uclog-target0` by default, so a plain build works (-S uclog-target1 for an image
# whose logs are told apart as target 1, such as an app behind a bootloader that also logs).
if(NOT "${SNIPPET}" MATCHES "uclog-target")
  set(SNIPPET uclog-target0 ${SNIPPET} CACHE STRING "" FORCE)
endif()

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(uclog_example)

target_sources(app PRIVATE src/main.c)
