# ucLog test firmware for the NUCLEO-L452RE-P (STM32L452RE, Cortex-M4F).
#
#   make            build build/app.elf (with .logdata_cbor), app.bin, app.logdata
#   make flash      program over the ST-LINK (STM32_Programmer_CLI)
#   make run        flash, then `uclog term` on the ST-LINK VCP
#   make clean
#
# Toolchain: arm-none-eabi-gcc on PATH, or STM32CubeCLT (CUBECLT below).
# STM32CubeL4 (CMSIS, LL headers, startup, system_stm32l4xx.c): STM32CUBE_L4 below, as installed
# by STM32CubeMX/STM32CubeIDE, or a checkout of github.com/STMicroelectronics/STM32CubeL4.

CUBECLT ?= /opt/ST/STM32CubeCLT_1.20.0
ifeq ($(shell command -v arm-none-eabi-gcc 2>/dev/null),)
  CROSS ?= $(CUBECLT)/GNU-tools-for-STM32/bin/arm-none-eabi-
else
  CROSS ?= arm-none-eabi-
endif
CC      := $(CROSS)gcc
OBJCOPY := $(CROSS)objcopy
OBJDUMP := $(CROSS)objdump
SIZE    := $(CROSS)size
PROG    ?= $(CUBECLT)/STM32CubeProgrammer/bin/STM32_Programmer_CLI
PYTHON  ?= python3
UCLOG   ?= ../../native/target/release/uclog
DEVICE  ?= auto

STM32CUBE_L4 ?= $(HOME)/STM32Cube/Repository/STM32Cube_FW_L4_V1.18.2
CMSIS_DEV := $(STM32CUBE_L4)/Drivers/CMSIS/Device/ST/STM32L4xx

UCLOG_DIR  := ../uclog
UCCOMM_DIR := ../uccomm
BUILD      := build

# Paths as given here become __FILE__, i.e. the file names shown with each log.
SRCS := src/main.c \
        src/drivers/sensor.c \
        src/sh/syscalls.c \
        src/sh/semihost_test.c \
        board/board.c \
        board/dma.c \
        board/gpio.c \
        board/uart.c \
        $(UCLOG_DIR)/log.c \
        $(UCCOMM_DIR)/cb.c \
        $(UCCOMM_DIR)/cobs.c \
        $(UCCOMM_DIR)/cbor.c \
        $(UCCOMM_DIR)/crc32c.c
CUBE_SRCS := $(CMSIS_DEV)/Source/Templates/system_stm32l4xx.c \
             $(CMSIS_DEV)/Source/Templates/gcc/startup_stm32l452xx.s

CPU    := -mcpu=cortex-m4 -mthumb -mfpu=fpv4-sp-d16 -mfloat-abi=hard -mfp16-format=ieee
CFLAGS := $(CPU) -std=gnu11 -Os -g3 -Wall -Wextra -Wno-override-init \
          -Wno-unused-parameter -ffunction-sections -fdata-sections \
          -fno-eliminate-unused-debug-types \
          -DSTM32L452xx -Isrc -Iboard -I$(UCLOG_DIR) -I$(UCCOMM_DIR) \
          -I$(STM32CUBE_L4)/Drivers/CMSIS/Include -I$(CMSIS_DEV)/Include \
          -I$(STM32CUBE_L4)/Drivers/STM32L4xx_HAL_Driver/Inc -MMD -MP
LDFLAGS := $(CPU) -Tboard/stm32l452re.ld -L$(UCLOG_DIR) -Wl,--gc-sections -Wl,-Map=$(BUILD)/app.map \
           --specs=nano.specs --specs=nosys.specs -u _printf_float -Wl,--no-warn-rwx-segments
LDLIBS := -lm

OBJS := $(patsubst %.c,$(BUILD)/obj/%.o,$(subst ../,,$(SRCS))) \
        $(BUILD)/obj/cube/system_stm32l4xx.o $(BUILD)/obj/cube/startup_stm32l452xx.o

all: $(BUILD)/app.bin

$(BUILD)/obj/%.o: %.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/obj/uclog/%.o: $(UCLOG_DIR)/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/obj/uccomm/%.o: $(UCCOMM_DIR)/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/obj/cube/%.o: $(CMSIS_DEV)/Source/Templates/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/obj/cube/%.o: $(CMSIS_DEV)/Source/Templates/gcc/%.s
	@mkdir -p $(dir $@)
	$(CC) $(CPU) -x assembler-with-cpp -c $< -o $@

# 1. link (the .apphash field is zero)
# 2. app_hash = SHA-512 of that image; write it into .apphash
# 3. logdata from the ELF; embed it as .logdata_cbor; save <app_hash>.logdata too
$(BUILD)/app.elf: $(OBJS) board/stm32l452re.ld $(UCLOG_DIR)/log.ld $(UCLOG_DIR)/tools/logdata.py
	$(CC) $(LDFLAGS) $(OBJS) $(LDLIBS) -o $@.tmp
	$(OBJCOPY) -O binary $@.tmp $(BUILD)/app.unhashed.bin
	$(PYTHON) $(UCLOG_DIR)/tools/apphash.py $(BUILD)/app.unhashed.bin -o $(BUILD)/app.hash > $(BUILD)/app.hash.txt
	$(OBJCOPY) --update-section .apphash=$(BUILD)/app.hash $@.tmp
	$(PYTHON) $(UCLOG_DIR)/tools/logdata.py --objdump $(OBJDUMP) $@.tmp -o $(BUILD)/app.logdata
	$(OBJCOPY) --add-section .logdata_cbor=$(BUILD)/app.logdata --set-section-flags .logdata_cbor=noload,readonly $@.tmp $@
	@rm -f $@.tmp
	cp $(BUILD)/app.logdata $(BUILD)/$$(cat $(BUILD)/app.hash.txt).logdata
	$(SIZE) $@

$(BUILD)/app.bin: $(BUILD)/app.elf
	$(OBJCOPY) -O binary $< $@

flash: $(BUILD)/app.bin
	$(PROG) -c port=SWD mode=UR -w $(BUILD)/app.bin 0x08000000 -v -rst

run: flash
	@mkdir -p $(BUILD)/files
	$(UCLOG) term -d $(DEVICE) -l $(BUILD)/app.elf --files-dir $(BUILD)/files

clean:
	rm -rf $(BUILD)

.PHONY: all flash run clean
-include $(OBJS:.o=.d)
