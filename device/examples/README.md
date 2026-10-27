# Examples

The smallest apps that use the library, each on the NUCLEO-L452RE-P (the ST-LINK's virtual COM
port at 1 Mbaud). Both do the same: a log every second, a warning every tenth, and a terminal
(port 61) that takes `led on`, `led off` and `led blink`. The device CI builds them; they were
checked on the board with `uclog`, the hub and the Python library.

| Directory | Build | |
|---|---|---|
| [baremetal/](baremetal/) | `make`, or `cmake -B build -DCMAKE_TOOLCHAIN_FILE=cortex-m4f.cmake && cmake --build build` | The port for the board in `port.c` (CMSIS registers only: USART2 with DMA, SysTick), a linker script with what ucLog needs, and the same sources built through `uclog.mk` and through CMake |
| [zephyr/](zephyr/) | `west build -b nucleo_l452re/stm32l452xx/p examples/zephyr -- -DZEPHYR_EXTRA_MODULES=<this directory's parent>` | Zephyr's `LOG_INF()` as ucLog logs; the link from the devicetree (`boards/`, also the nRF52840-DK); the `uclog-target0` snippet |

The bare-metal example needs Arm GCC (`arm-none-eabi-gcc`, or STM32CubeCLT), Python 3 and the
STM32L4 CMSIS files (an STM32CubeL4 package, or `github.com/STMicroelectronics/STM32CubeL4` with
its submodules; set `STM32CUBE_L4`). `make flash` programs the board; then connect with any host,
for example `uclog tui -d auto`. The build puts the logdata in the hosts' cache, so they decode
the logs without being given the ELF.

The full test firmware, with every feature and more boards and links (USB CDC and HID, BLE), is
[firmware/](https://github.com/UnitCircleInc/uclog/tree/main/firmware) in the ucLog repository.
