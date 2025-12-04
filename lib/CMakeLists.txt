# © 2025 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

zephyr_library()

zephyr_library_sources(
  cb.c
  cobs.c
  cbor.c
)

zephyr_library_sources_ifdef(CONFIG_UC_LOG log.c)
zephyr_library_sources_ifdef(CONFIG_UC_FAULT fault.c)
zephyr_library_sources_ifdef(CONFIG_UC_LOG_SERVER logserver.c)
zephyr_library_sources_ifdef(CONFIG_UC_SYSCALLS syscalls.c)
zephyr_library_sources_ifdef(CONFIG_UC_SHELL shell.c)

if(CONFIG_UC_LOG)
  include(log.cmake)
  zephyr_linker_sources(SECTIONS log.ld)
endif()

if(CONFIG_UC_SIGNED_IMAGE)
  include(signature.cmake)
  zephyr_linker_sources(
    ROM_START
    SORT_KEY 0000
    signature.ld
  )

else()
  include(apphash.cmake)

  zephyr_linker_sources(SECTIONS apphash.ld)

endif()
