# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
#
# The device library built for the host (Arm64 Linux: ELF sections, and ucComm builds), driven by
# test_vectors.py:
#   cmake -S device/test/host -B build/host && cmake --build build/host
#   python3 device/test/host/test_vectors.py build/host/uclog_host_test

cmake_minimum_required(VERSION 3.20)
project(uclog_host_test C)
set(CMAKE_C_STANDARD 11)
add_compile_options(-Wall -Wextra -Werror -Wno-unused-parameter)
# ucComm (uccomm/, v3) does not build on x86: cbor.h's __fp16 test checks __x86_64_ (sic), and
# cbor_vpack() copies a va_list into a struct, which x86-64's array va_list does not allow. An
# Arm64 host (Linux, e.g. GitHub's ubuntu-24.04-arm) builds it as is.
if(CMAKE_SYSTEM_PROCESSOR MATCHES "x86_64|AMD64|i.86")
  message(FATAL_ERROR "the host test needs an Arm host: ucComm does not build on x86 (see above)")
endif()
add_subdirectory(../.. uclog)
add_executable(uclog_host_test driver.c host_port.c)
target_link_libraries(uclog_host_test PRIVATE uclog)
