# A CMake toolchain file for this example: Arm GCC for a Cortex-M4 with its FPU (hard float).
# The CPU flags go into every target, the library's too: all objects must agree on the float ABI.
#
#   cmake -B build -DCMAKE_TOOLCHAIN_FILE=cortex-m4f.cmake

set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_SYSTEM_PROCESSOR arm)
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY) # no startup code for test programs

# arm-none-eabi-gcc on PATH, else STM32CubeCLT's.
find_program(ARM_GCC arm-none-eabi-gcc PATHS /opt/ST/STM32CubeCLT_1.22.0/GNU-tools-for-STM32/bin REQUIRED)
set(CMAKE_C_COMPILER ${ARM_GCC})
set(CMAKE_ASM_COMPILER ${ARM_GCC})

set(CPU "-mcpu=cortex-m4 -mthumb -mfpu=fpv4-sp-d16 -mfloat-abi=hard")
set(CMAKE_C_FLAGS_INIT "${CPU}")
set(CMAKE_ASM_FLAGS_INIT "${CPU}")
set(CMAKE_EXE_LINKER_FLAGS_INIT "${CPU} --specs=nano.specs --specs=nosys.specs")
