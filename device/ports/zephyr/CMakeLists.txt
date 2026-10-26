# © 2025 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

if(CONFIG_UC_LOG)
  zephyr_library()
  zephyr_library_sources(${UCLOG_ROOT_DIR}/src/log.c port.c)
  zephyr_library_sources_ifdef(CONFIG_UC_SYSCALLS ${UCLOG_ROOT_DIR}/src/semihost.c semihost_fs.c)
  zephyr_library_sources_ifdef(CONFIG_UC_SHELL shell.c)

  include(${UCLOG_ROOT_DIR}/cmake/log.cmake)
  zephyr_linker_sources(SECTIONS ${UCLOG_ROOT_DIR}/cmake/log.ld)

  if(CONFIG_UC_SIGNED_IMAGE)
    include(${UCLOG_ROOT_DIR}/cmake/signature.cmake)
    zephyr_linker_sources(ROM_START SORT_KEY 0000 ${UCLOG_ROOT_DIR}/cmake/signature.ld)
  else()
    include(${UCLOG_ROOT_DIR}/cmake/apphash.cmake)
    zephyr_linker_sources(SECTIONS ${UCLOG_ROOT_DIR}/cmake/apphash.ld)
  endif()
endif()
