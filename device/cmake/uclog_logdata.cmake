# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
#
# uclog_add_logdata(<target>) for bare-metal CMake projects (Zephyr has log.cmake and
# apphash.cmake). After the link:
#   1. the app hash (SHA-512 of the binary image, the .apphash field still zero) into .apphash;
#   2. the logdata from the ELF (tools/logdata.py), embedded as .logdata_cbor;
#   3. <target>.logdata, <target>.hash and <app_hash>.logdata next to the ELF, and a copy in the
#      hosts' cache (tools/cachelogdata.py).
# Needs CMAKE_OBJCOPY and CMAKE_OBJDUMP (a GNU toolchain) and Python 3.

set(UCLOG_TOOLS_DIR "${CMAKE_CURRENT_LIST_DIR}/../tools" CACHE INTERNAL "")

function(uclog_add_logdata target)
  find_package(Python3 REQUIRED COMPONENTS Interpreter)
  set(elf $<TARGET_FILE:${target}>)
  set(out $<TARGET_FILE_DIR:${target}>/${target})
  add_custom_command(TARGET ${target} POST_BUILD
    COMMAND ${CMAKE_OBJCOPY} -O binary ${elf} ${out}.unhashed.bin
    COMMAND ${Python3_EXECUTABLE} ${UCLOG_TOOLS_DIR}/hash.py -f -i ${out}.unhashed.bin -o ${out}.hash
    COMMAND ${CMAKE_OBJCOPY} --update-section .apphash=${out}.hash ${elf}
    COMMAND ${Python3_EXECUTABLE} ${UCLOG_TOOLS_DIR}/logdata.py --objdump ${CMAKE_OBJDUMP} ${elf} -o ${out}.logdata
    COMMAND ${CMAKE_OBJCOPY} --add-section .logdata_cbor=${out}.logdata
            --set-section-flags .logdata_cbor=noload,readonly ${elf}
    COMMAND ${Python3_EXECUTABLE} ${UCLOG_TOOLS_DIR}/cachelogdata.py --bin ${out}.unhashed.bin
            --logdata ${out}.logdata -f --hashfile ${out}.hash --copy-to $<TARGET_FILE_DIR:${target}>
    VERBATIM)
endfunction()
