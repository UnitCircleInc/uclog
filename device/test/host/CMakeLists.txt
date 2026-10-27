# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
#
# The device library built for the host (Linux: it needs ELF sections), driven by test_vectors.py:
#   cmake -S device/test/host -B build/host && cmake --build build/host
#   python3 device/test/host/test_vectors.py build/host/uclog_host_test

cmake_minimum_required(VERSION 3.20)
project(uclog_host_test C)
set(CMAKE_C_STANDARD 11)
add_compile_options(-Wall -Wextra -Werror -Wno-unused-parameter)
add_subdirectory(../.. uclog)
add_executable(uclog_host_test driver.c host_port.c)
target_link_libraries(uclog_host_test PRIVATE uclog)
