# © 2025 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

# Since this file is brought in via include(), we do the work in a
# function to avoid polluting the top-level scope.

function(zephyr_runner_file type path)
  # Property magic which makes west flash choose the signed build
  # output of a given type.
  set_target_properties(runners_yaml_props_target PROPERTIES "${type}_file" "${path}")
endfunction()

message(WARNING "UCLOG_ROOT_DIR: ${UCLOG_ROOT_DIR}")
message(WARNING "CMAKE_CURRENT_SOURCE_DIR: ${CMAKE_CURRENT_SOURCE_DIR}")
message(WARNING "CMAKE_CURRENT_LIST_DIR: ${CMAKE_CURRENT_LIST_DIR}")

function(zephyr_hash_tasks)
  # Extensionless prefix of any output file.
  set(output ${ZEPHYR_BINARY_DIR}/${KERNEL_NAME})

  # List of additional build byproducts.
  set(byproducts)

  # Set up .bin outputs.
  if(CONFIG_BUILD_OUTPUT_BIN)
    list(APPEND byproducts ${output}.hash.bin)
    zephyr_runner_file(bin ${output}.hash.bin)
    set(BYPRODUCT_KERNEL_SIGNED_BIN_NAME "${output}.hash.bin"
        CACHE FILEPATH "Signed kernel bin file" FORCE
    )
  endif()

  # Set up .hex outputs.
  if(CONFIG_BUILD_OUTPUT_HEX)
    list(APPEND byproducts ${output}.hash.hex)
    zephyr_runner_file(hex ${output}.hash.hex)
    set(BYPRODUCT_KERNEL_SIGNED_HEX_NAME "${output}.hash.hex"
        CACHE FILEPATH "Signed kernel hex file" FORCE
    )
  endif()

  # CMake guarantees that multiple COMMANDs given to
  # add_custom_command() are run in order, so adding the 'west sign'
  # calls to the "extra_post_build_commands" property ensures they run
  # after the commands which generate the unsigned versions.
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands COMMAND
    echo "Adding apphash to ${output}.elf")
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands COMMAND
    ${UCLOG_ROOT_DIR}/scripts/hash.py -f -i ${output}.bin -o ${output}.hash)
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands COMMAND
    ${CMAKE_OBJCOPY} --update-section .apphash=${output}.hash ${output}.elf ${output}.hash.elf)
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands COMMAND
    ${CMAKE_OBJCOPY} -O binary --gap-fill 0xff --keep-section=.apphash --remove-section=.comment --remove-section=COMMON --remove-section=.eh_frame ${output}.hash.elf ${output}.hash.bin)
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands COMMAND
    ${CMAKE_OBJCOPY} -O ihex --gap-fill 0xff --keep-section=.apphash --remove-section=.comment --remove-section=COMMON --remove-section=.eh_frame ${output}.hash.elf ${output}.hash.hex)
  set_property(GLOBAL APPEND PROPERTY extra_post_build_byproducts ${byproducts})
endfunction()

zephyr_hash_tasks()
