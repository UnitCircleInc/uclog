# A minimal ucLog app on the NUCLEO-L452RE-P (bare metal, CMake; the Makefile builds the same).
#
#   cmake -B build -DCMAKE_TOOLCHAIN_FILE=cortex-m4f.cmake [-DSTM32CUBE_L4=...]
#   cmake --build build      # build/example.elf (logdata embedded), example.bin; the logdata
#                            # also goes to the hosts' cache
#
# Needs CMake 3.20, Arm GCC, Python 3, and ST's CMSIS files for the STM32L4 (as in the Makefile).

cmake_minimum_required(VERSION 3.20)
project(uclog_example C ASM)

set(STM32CUBE_L4 "$ENV{HOME}/STM32Cube/Repository/STM32Cube_FW_L4_V1.18.2" CACHE PATH "STM32CubeL4")
set(CMSIS_DEV ${STM32CUBE_L4}/Drivers/CMSIS/Device/ST/STM32L4xx)

# For every target, the library's too. -g: logdata.py reads the format strings' argument types
# (and enums) from the debug info.
add_compile_options(-Os -g -ffunction-sections -fdata-sections)

# 1. The library: the uclog and uclog_syscalls targets, and uclog_add_logdata(). In your project,
#    point this at the device library, or fetch it:
#      FetchContent_Declare(uclog GIT_REPOSITORY https://github.com/UnitCircleInc/uclog-device GIT_TAG v4)
#      FetchContent_MakeAvailable(uclog)
add_subdirectory(../.. uclog)

add_executable(example
  main.c
  port.c
  ${CMSIS_DEV}/Source/Templates/system_stm32l4xx.c
  ${CMSIS_DEV}/Source/Templates/gcc/startup_stm32l452xx.s
)
set_target_properties(example PROPERTIES SUFFIX .elf)
target_compile_definitions(example PRIVATE STM32L452xx)
target_include_directories(example PRIVATE ${STM32CUBE_L4}/Drivers/CMSIS/Include ${CMSIS_DEV}/Include)
# CMake compiles absolute paths: map them, so logs show main.c rather than the path on this machine
# (the library maps its own to uclog/...).
target_compile_options(example PRIVATE -Wall -Wextra -Wno-unused-parameter -ffile-prefix-map=${CMAKE_CURRENT_SOURCE_DIR}/=)
# uclog_syscalls: newlib's system calls over the link (semihost), so stdout, stdin and files reach
# the host. The linker script INCLUDEs the library's log.ld (uclog adds its directory to -L).
target_link_libraries(example PRIVATE uclog uclog_syscalls)
target_link_options(example PRIVATE -T${CMAKE_CURRENT_SOURCE_DIR}/stm32l452re.ld -Wl,--gc-sections -Wl,--no-warn-rwx-segments)
set_target_properties(example PROPERTIES LINK_DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/stm32l452re.ld)

# 2. After the link: the app hash into .apphash, the logdata embedded (.logdata_cbor), and
#    <app_hash>.logdata next to the ELF and in the hosts' cache.
uclog_add_logdata(example)

add_custom_command(TARGET example POST_BUILD
  COMMAND ${CMAKE_OBJCOPY} -O binary $<TARGET_FILE:example> $<TARGET_FILE_DIR:example>/example.bin
  VERBATIM)
