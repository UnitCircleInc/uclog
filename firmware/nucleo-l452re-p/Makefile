# ucLog test firmware for the NUCLEO-L452RE-P (STM32L452RE, Cortex-M4F).
#
#   make            build build/app.elf (with .logdata_cbor), app.bin, app.logdata;
#                   copies <app_hash>.logdata to $(UCLOG_CACHE)
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
# The hosts find logdata here by the device's app_hash (same default as uclog and the Python library)
UCLOG_CACHE ?= $(or $(UCLOG_CACHE_DIR),$(XDG_CACHE_HOME:%=%/uclog),$(HOME)/.cache/uclog)
UCLOG   ?= ../../native/target/release/uclog
DEVICE  ?= auto

STM32CUBE_L4 ?= $(HOME)/STM32Cube/Repository/STM32Cube_FW_L4_V1.18.2
CMSIS_DEV := $(STM32CUBE_L4)/Drivers/CMSIS/Device/ST/STM32L4xx

DEVICE_DIR := ../../device
UCCOMM_DIR := ../uccomm
APP_DIR    := ../app
BUILD      := build

# Paths as given here become __FILE__, i.e. the file names shown with each log; the shared test
# app (../app) is shown as src/ (-ffile-prefix-map below).
SRCS := $(APP_DIR)/main.c \
        $(APP_DIR)/drivers/sensor.c \
        $(DEVICE_DIR)/src/syscalls.c \
        $(DEVICE_DIR)/src/semihost.c \
        $(APP_DIR)/sh/semihost_test.c \
        board/board.c \
        board/dma.c \
        board/gpio.c \
        board/uart.c \
        $(DEVICE_DIR)/src/log.c \
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
          -ffile-prefix-map=$(APP_DIR)/=src/ -ffile-prefix-map=$(DEVICE_DIR)/=uclog/ \
          -DSTM32L452xx -I$(APP_DIR) -Iboard -I$(DEVICE_DIR)/include -I$(UCCOMM_DIR) \
          -I$(STM32CUBE_L4)/Drivers/CMSIS/Include -I$(CMSIS_DEV)/Include \
          -I$(STM32CUBE_L4)/Drivers/STM32L4xx_HAL_Driver/Inc -MMD -MP
LDFLAGS := $(CPU) -Tboard/stm32l452re.ld -L$(DEVICE_DIR)/ports/baremetal -Wl,--gc-sections -Wl,-Map=$(BUILD)/app.map \
           --specs=nano.specs --specs=nosys.specs -u _printf_float -Wl,--no-warn-rwx-segments
LDLIBS := -lm

OBJS := $(patsubst %.c,$(BUILD)/obj/%.o,$(subst ../,,$(SRCS))) \
        $(BUILD)/obj/cube/system_stm32l4xx.o $(BUILD)/obj/cube/startup_stm32l452xx.o

all: $(BUILD)/app.bin

# Rebuild every object when the compiler or its flags change (a -D buffer size, say): the stamp
# file is rewritten only then.
FLAGS_STAMP := $(BUILD)/cflags.txt
$(shell mkdir -p $(BUILD); echo '$(CC) $(CFLAGS)' | cmp -s - $(FLAGS_STAMP) || echo '$(CC) $(CFLAGS)' > $(FLAGS_STAMP))
$(OBJS): $(FLAGS_STAMP)

$(BUILD)/obj/%.o: %.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/obj/app/%.o: $(APP_DIR)/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/obj/device/%.o: $(DEVICE_DIR)/%.c
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
$(BUILD)/app.elf: $(OBJS) board/stm32l452re.ld $(DEVICE_DIR)/ports/baremetal/log.ld $(DEVICE_DIR)/tools/logdata.py \
             $(DEVICE_DIR)/tools/hash.py
	$(CC) $(LDFLAGS) $(OBJS) $(LDLIBS) -o $@.tmp
	$(OBJCOPY) -O binary $@.tmp $(BUILD)/app.unhashed.bin
	$(PYTHON) $(DEVICE_DIR)/tools/hash.py -f -i $(BUILD)/app.unhashed.bin -o $(BUILD)/app.hash > $(BUILD)/app.hash.txt
	$(OBJCOPY) --update-section .apphash=$(BUILD)/app.hash $@.tmp
	$(PYTHON) $(DEVICE_DIR)/tools/logdata.py --objdump $(OBJDUMP) $@.tmp -o $(BUILD)/app.logdata
	$(OBJCOPY) --add-section .logdata_cbor=$(BUILD)/app.logdata --set-section-flags .logdata_cbor=noload,readonly $@.tmp $@
	@rm -f $@.tmp
	cp $(BUILD)/app.logdata $(BUILD)/$$(cat $(BUILD)/app.hash.txt).logdata
	@mkdir -p $(UCLOG_CACHE) && cp $(BUILD)/app.logdata $(UCLOG_CACHE)/$$(cat $(BUILD)/app.hash.txt).logdata
	$(SIZE) $@

$(BUILD)/app.bin: $(BUILD)/app.elf
	$(OBJCOPY) -O binary $< $@

# The ST-LINK of this board (several boards may be connected): STLINK_SN, else the probe whose
# board name is BOARD_PROBE.
BOARD_PROBE ?= NUCLEO-L452RE-P
STLINK_SN ?= $(shell $(PROG) -l st-link 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | \
               awk '/ST-LINK SN/ {sn=$$NF} /Board Name/ && $$NF=="$(BOARD_PROBE)" {print sn; exit}')

flash: $(BUILD)/app.bin
	@test -n "$(STLINK_SN)" || { echo "no $(BOARD_PROBE) ST-LINK found (set STLINK_SN)"; exit 1; }
	$(PROG) -c port=SWD mode=UR sn=$(STLINK_SN) -w $(BUILD)/app.bin 0x08000000 -v -rst

run: flash
	@mkdir -p $(BUILD)/files
	$(UCLOG) term -d $(DEVICE) -l $(BUILD)/app.elf --files-dir $(BUILD)/files

clean:
	rm -rf $(BUILD)

.PHONY: all flash run clean
-include $(OBJS:.o=.d)
