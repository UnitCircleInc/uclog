# © 2023 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

# Since this file is brought in via include(), we do the work in a
# function to avoid polluting the top-level scope.

function(zephyr_runner_file type path)
  # Property magic which makes west flash choose the signed build
  # output of a given type.
  set_target_properties(runners_yaml_props_target PROPERTIES "${type}_file" "${path}")
endfunction()

function(zephyr_sbl_tasks)
  # Extensionless prefix of any output file.
  set(output ${ZEPHYR_BINARY_DIR}/${KERNEL_NAME})

  # List of additional build byproducts.
  set(byproducts)

  # Set up .bin outputs.
  if(CONFIG_BUILD_OUTPUT_BIN)
    list(APPEND byproducts ${output}.signed.bin)
    zephyr_runner_file(bin ${output}.signed.bin)
    set(BYPRODUCT_KERNEL_SIGNED_BIN_NAME "${output}.signed.bin"
        CACHE FILEPATH "Signed kernel bin file" FORCE
    )
  endif()

  # Set up .hex outputs.
  if(CONFIG_BUILD_OUTPUT_HEX)
    list(APPEND byproducts ${output}.signed.hex)
    zephyr_runner_file(hex ${output}.signed.hex)
    set(BYPRODUCT_KERNEL_SIGNED_HEX_NAME "${output}.signed.hex"
        CACHE FILEPATH "Signed kernel hex file" FORCE
    )
  endif()

  # CMake guarantees that multiple COMMANDs given to
  # add_custom_command() are run in order, so adding the 'west sign'
  # calls to the "extra_post_build_commands" property ensures they run
  # after the commands which generate the unsigned versions.
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands COMMAND
    echo "Signing ${output}.elf CERT $ENV{SBL_NEB_CERT}")
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands COMMAND
    ${APP_ROOT_DIR}/scripts/sbl.py sign --key $ENV{SBL_NEB} --code ${output}.hex --cert ${APP_ROOT_DIR}/$ENV{SBL_NEB_CERT} ${output}.signed.hex)
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands COMMAND
    ${APP_ROOT_DIR}/scripts/sbl.py sign --key $ENV{SBL_NEB} --code ${output}.bin --cert ${APP_ROOT_DIR}/$ENV{SBL_NEB_CERT} ${output}.signed.bin)
  set_property(GLOBAL APPEND PROPERTY extra_post_build_byproducts ${byproducts})
endfunction()

zephyr_sbl_tasks()
