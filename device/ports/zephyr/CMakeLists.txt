# © 2025 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

if(CONFIG_UC_LOG)
  zephyr_library()
  zephyr_library_sources(${UCLOG_ROOT_DIR}/src/log.c port.c)
  if(NOT CONFIG_UCCOMM) # no ucComm module in the workspace: the pinned copy
    zephyr_include_directories(${UCLOG_ROOT_DIR}/uccomm)
    zephyr_library_sources(${UCLOG_ROOT_DIR}/uccomm/cb.c ${UCLOG_ROOT_DIR}/uccomm/cobs.c
                           ${UCLOG_ROOT_DIR}/uccomm/cbor.c ${UCLOG_ROOT_DIR}/uccomm/crc32c.c)
  endif()
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
