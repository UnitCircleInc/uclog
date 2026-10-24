# ucLog test firmware for the NUCLEO-C071RB (STM32C071RB, Cortex-M0+, 128 KiB flash, 24 KiB RAM).
# Three links at once: USART2 (ST-LINK virtual COM port), USB CDC-ACM and USB HID (the user USB
# connector); the first one a host talks on is used.
#
#   make            build build/app.elf (with .logdata_cbor), app.bin, app.logdata
#   make flash      program over this board's ST-LINK (STM32_Programmer_CLI)
#   make clean
#
# Toolchain: arm-none-eabi-gcc on PATH, or STM32CubeCLT (CUBECLT below).
# STM32CubeC0 (CMSIS core and device headers, LL headers, system_stm32c0xx.c): STM32CUBE_C0 below,
# an STM32CubeC0 package as installed by STM32CubeMX, or a checkout of
# github.com/STMicroelectronics/STM32CubeC0 with its submodules. Zephyr's hal_stm32 module directory
# modules/hal/stm32/stm32cube/stm32c0xx also works, with CMSIS_CORE set to a CMSIS 5 Include/.

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

STM32CUBE_C0 ?= $(HOME)/STM32Cube/Repository/STM32Cube_FW_C0_V1.4.1
CMSIS_CORE   ?= $(STM32CUBE_C0)/Drivers/CMSIS/Include
ifneq ($(wildcard $(STM32CUBE_C0)/soc/stm32c0xx.h),)
  # Zephyr hal_stm32 layout
  C0_DEVICE := $(STM32CUBE_C0)/soc
  C0_SYSTEM := $(STM32CUBE_C0)/soc/system_stm32c0xx.c
  C0_LL     := $(STM32CUBE_C0)/drivers/include
else
  C0_DEVICE := $(STM32CUBE_C0)/Drivers/CMSIS/Device/ST/STM32C0xx/Include
  C0_SYSTEM := $(STM32CUBE_C0)/Drivers/CMSIS/Device/ST/STM32C0xx/Source/Templates/system_stm32c0xx.c
  C0_LL     := $(STM32CUBE_C0)/Drivers/STM32C0xx_HAL_Driver/Inc
endif

UCLOG_DIR  := ../uclog
UCCOMM_DIR := ../uccomm
APP_DIR    := ../app
BUILD      := build

# Paths as given here become __FILE__, i.e. the file names shown with each log; the shared test
# app (../app) is shown as src/ (-ffile-prefix-map below).
SRCS := $(APP_DIR)/main.c \
        $(APP_DIR)/drivers/sensor.c \
        $(APP_DIR)/sh/syscalls.c \
        $(APP_DIR)/sh/semihost_test.c \
        board/board.c \
        board/startup.c \
        board/uart.c \
        board/usb.c \
        $(UCLOG_DIR)/log.c \
        $(UCCOMM_DIR)/cb.c \
        $(UCCOMM_DIR)/cobs.c \
        $(UCCOMM_DIR)/cbor.c \
        $(UCCOMM_DIR)/crc32c.c

# 24 KiB of RAM: smaller buffers than the L452 (log ring, semihost requests, the selftest's file).
RAM_OPTS := -DLOG_TX_BUF_SIZE=4096 -DSH_BUF_SIZE=512 -DSH_BIG_FILE=1200

CPU    := -mcpu=cortex-m0plus -mthumb -mfloat-abi=soft -mfp16-format=ieee
CFLAGS := $(CPU) -std=gnu11 -Os -g3 -Wall -Wextra -Wno-override-init \
          -Wno-unused-parameter -ffunction-sections -fdata-sections \
          -fno-eliminate-unused-debug-types -ffile-prefix-map=$(APP_DIR)/=src/ \
          -DSTM32C071xx $(RAM_OPTS) -I$(APP_DIR) -Iboard -I$(UCLOG_DIR) -I$(UCCOMM_DIR) \
          -I$(CMSIS_CORE) -I$(C0_DEVICE) -I$(C0_LL) -MMD -MP
LDFLAGS := $(CPU) -Tboard/stm32c071rb.ld -L$(UCLOG_DIR) -Wl,--gc-sections -Wl,-Map=$(BUILD)/app.map \
           --specs=nano.specs --specs=nosys.specs -u _printf_float -Wl,--no-warn-rwx-segments
LDLIBS := -lm

OBJS := $(patsubst %.c,$(BUILD)/obj/%.o,$(subst ../,,$(SRCS))) $(BUILD)/obj/cube/system_stm32c0xx.o

all: $(BUILD)/app.bin

$(BUILD)/obj/%.o: %.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/obj/app/%.o: $(APP_DIR)/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/obj/uclog/%.o: $(UCLOG_DIR)/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/obj/uccomm/%.o: $(UCCOMM_DIR)/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/obj/cube/system_stm32c0xx.o: $(C0_SYSTEM)
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

# 1. link (the .apphash field is zero)
# 2. app_hash = SHA-512 of that image; write it into .apphash
# 3. logdata from the ELF; embed it as .logdata_cbor; save <app_hash>.logdata too
$(BUILD)/app.elf: $(OBJS) board/stm32c071rb.ld $(UCLOG_DIR)/log.ld $(UCLOG_DIR)/tools/logdata.py
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

# The ST-LINK of this board (several boards may be connected): STLINK_SN, else the probe whose
# board name is BOARD_PROBE.
BOARD_PROBE ?= NUCLEO-C071RB
STLINK_SN ?= $(shell $(PROG) -l st-link 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | \
               awk '/ST-LINK SN/ {sn=$$NF} /Board Name/ && $$NF=="$(BOARD_PROBE)" {print sn; exit}')

flash: $(BUILD)/app.bin
	@test -n "$(STLINK_SN)" || { echo "no $(BOARD_PROBE) ST-LINK found (set STLINK_SN)"; exit 1; }
	$(PROG) -c port=SWD mode=UR sn=$(STLINK_SN) -w $(BUILD)/app.bin 0x08000000 -v -rst

clean:
	rm -rf $(BUILD)

.PHONY: all flash clean
-include $(OBJS:.o=.d)
