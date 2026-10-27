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
  # Logs name this library's files uclog/... (as on bare metal): no absolute paths, which could
  # contain ':' on Windows (log.h).
  zephyr_compile_options(-fmacro-prefix-map=${UCLOG_ROOT_DIR}=uclog)
  add_subdirectory(drivers)
  add_subdirectory(ports/zephyr)
  return()
endif()

cmake_minimum_required(VERSION 3.20)

add_library(uclog STATIC
  src/log.c
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
