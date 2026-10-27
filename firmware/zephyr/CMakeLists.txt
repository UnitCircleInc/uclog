# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
#
# The test firmware (../app) on Zephyr 4.x, with the ucLog device library as a Zephyr module
# (../../device) and ucComm. See README.md.

cmake_minimum_required(VERSION 3.20.0)
# Always, besides any boards/<board>.overlay (which picks the ucLog link).
list(APPEND EXTRA_DTC_OVERLAY_FILE ${CMAKE_CURRENT_SOURCE_DIR}/logdata.overlay)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(uclog_test)

cmake_path(SET APP_DIR NORMALIZE ${CMAKE_CURRENT_SOURCE_DIR}/../app) # normalised: the prefix map below matches it
target_sources(app PRIVATE
  ${APP_DIR}/main.c
  ${APP_DIR}/bench.c
  ${APP_DIR}/drivers/sensor.c
  ${APP_DIR}/sh/semihost_test.c
  src/board.c
)
target_include_directories(app PRIVATE ${APP_DIR} src)
# Logs (and {sym}, from the DWARF) name the shared app's files src/... as on the bare-metal boards.
target_compile_options(app PRIVATE -ffile-prefix-map=${APP_DIR}/=src/ -ffile-prefix-map=${CMAKE_CURRENT_SOURCE_DIR}/src/=board/)
# The shared app logs floats on purpose (they are promoted to double, as the test expects).
set_source_files_properties(${APP_DIR}/main.c ${APP_DIR}/bench.c ${APP_DIR}/drivers/sensor.c
  PROPERTIES COMPILE_OPTIONS -Wno-double-promotion)
