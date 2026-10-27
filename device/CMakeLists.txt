# © 2025 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
#
# The ucLog device library.
#
# - Zephyr: a Zephyr module (zephyr/module.yml, or ../zephyr/module.yml in the ucLog repository):
#   the library, its Zephyr port and the ucuart/ucusb drivers (README.md).
# - Any other CMake project (bare metal): add_subdirectory() this directory, or FetchContent
#   (SOURCE_SUBDIR device from the ucLog repository; the uclog-device mirror has it at the root):
#
#     target_link_libraries(app PRIVATE uclog uclog_syscalls)  # uclog_syscalls: newlib semihost
#     uclog_add_logdata(app)                                     # app hash, logdata, host cache
#
#   and in the linker script, INCLUDE log.ld (in UCLOG_LINKER_DIR) and a 64-byte .apphash section.

set(UCLOG_ROOT_DIR "${CMAKE_CURRENT_SOURCE_DIR}")

if(COMMAND zephyr_library)
  zephyr_include_directories(include)
  # The file names logs show (__FILE__): relative, as on bare metal, and never absolute paths,
  # which could contain ':' on Windows (log.h). Zephyr maps the app to CMAKE_SOURCE_DIR/ and the
  # workspace to WEST_TOPDIR/; the last map that matches wins, so from here: Zephyr's files are
  # zephyr/... (and modules/...), the app's src/main.c, and this library's uclog/... (last: it may
  # be inside the app).
  # The same maps name the variables' files in the logdata (tools/logdata.py, cmake/log.cmake).
  if(WEST_TOPDIR)
    set(UCLOG_PREFIX_MAPS ${WEST_TOPDIR}/=)
  else()
    set(UCLOG_PREFIX_MAPS ${ZEPHYR_BASE}/=zephyr/)
  endif()
  list(APPEND UCLOG_PREFIX_MAPS ${CMAKE_SOURCE_DIR}/= ${UCLOG_ROOT_DIR}=uclog)
  foreach(map ${UCLOG_PREFIX_MAPS})
    zephyr_compile_options(-fmacro-prefix-map=${map})
  endforeach()
  add_subdirectory(drivers)
  add_subdirectory(ports/zephyr)
  return()
endif()

cmake_minimum_required(VERSION 3.20)

add_library(uclog STATIC
  src/log.c
  src/log_plot.c
  src/semihost.c
  uccomm/cb.c
  uccomm/cobs.c
  uccomm/cbor.c
  uccomm/crc32c.c
)
target_include_directories(uclog PUBLIC include uccomm)
target_compile_options(uclog PRIVATE -ffile-prefix-map=${UCLOG_ROOT_DIR}/=uclog/)
# ucComm's cbor.h uses __fp16, which Arm GCC only has with an explicit format.
include(CheckCSourceCompiles)
check_c_source_compiles("#if !defined(__arm__) || !defined(__GNUC__) || defined(__clang__)
#error
#endif
int uclog_arm_gcc;" UCLOG_ARM_GCC)
if(UCLOG_ARM_GCC)
  target_compile_options(uclog PRIVATE -mfp16-format=ieee)
endif()

# newlib's syscalls over semihost: an object library, so they replace nosys's whatever the link
# order.
add_library(uclog_syscalls OBJECT EXCLUDE_FROM_ALL src/syscalls.c)
target_link_libraries(uclog_syscalls PUBLIC uclog)
target_compile_options(uclog_syscalls PRIVATE -ffile-prefix-map=${UCLOG_ROOT_DIR}/=uclog/)

set(UCLOG_LINKER_DIR "${UCLOG_ROOT_DIR}/ports/baremetal" CACHE INTERNAL "log.ld for INCLUDE")
target_link_options(uclog INTERFACE "-L${UCLOG_LINKER_DIR}")

include(${UCLOG_ROOT_DIR}/cmake/uclog_logdata.cmake)
