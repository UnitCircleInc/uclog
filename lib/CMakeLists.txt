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

if(CONFIG_UC_LOG)
  include(log.cmake)
  zephyr_linker_sources(SECTIONS log.ld)
endif()

if(CONFIG_UC_SIGNED_IMAGE)
  zephyr_linker_sources(
    ROM_START
    SORT_KEY 0000
    signature.ld
  )

  set_target_properties(
    zephyr_property_target
    PROPERTIES SIGNING_SCRIPT ${CMAKE_CURRENT_LIST_DIR}/signature.cmake
  )

else()

if(CONFIG_UC_LOG_SAVE)
  zephyr_linker_sources(SECTIONS apphash.ld)

  set_target_properties(
    zephyr_property_target
    PROPERTIES SIGNING_SCRIPT ${CMAKE_CURRENT_LIST_DIR}/apphash.cmake
  )
endif()

endif()
