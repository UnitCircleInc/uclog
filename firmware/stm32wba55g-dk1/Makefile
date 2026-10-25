# ucLog test firmware for the STM32WBA55G-DK1 (STM32WBA55CG, Cortex-M33, 1 MiB flash, 128 KiB RAM).
# Two links at once: USART1 (ST-LINK virtual COM port) and BLE (protocol.md §4.2); the first one a
# host talks on is used.
#
#   make            build build/app.elf (with .logdata_cbor), app.bin, app.logdata
#   make flash      program over this board's ST-LINK (STM32_Programmer_CLI)
#   make clean
#
# Toolchain: arm-none-eabi-gcc on PATH, or STM32CubeCLT (CUBECLT below).
# STM32CubeWBA (CMSIS, HAL/LL, the BLE stack and link layer libraries, and the platform files of
# its BLE_p2pServer_Simplest example): STM32CUBE_WBA below, an STM32CubeWBA package as installed by
# STM32CubeMX.

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

STM32CUBE_WBA ?= $(HOME)/STM32Cube/Repository/STM32Cube_FW_WBA_V1.10.0
WBA_DEV  := $(STM32CUBE_WBA)/Drivers/CMSIS/Device/ST/STM32WBAxx
WBA_HAL  := $(STM32CUBE_WBA)/Drivers/STM32WBAxx_HAL_Driver
WPAN     := $(STM32CUBE_WBA)/Middlewares/ST/STM32_WPAN
# The chip-level BLE platform files (link layer and host stack glue) and their configuration come
# from ST's smallest BLE peripheral example; it is for the NUCLEO-WBA55CG, the same chip.
WBA_EX   := $(STM32CUBE_WBA)/Projects/NUCLEO-WBA55CG/Applications/BLE/BLE_p2pServer_Simplest

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
        board/ble.c \
        board/uart.c \
        $(UCLOG_DIR)/log.c \
        $(UCCOMM_DIR)/cb.c \
        $(UCCOMM_DIR)/cobs.c \
        $(UCCOMM_DIR)/cbor.c \
        $(UCCOMM_DIR)/crc32c.c

CUBE_SRCS := $(WBA_DEV)/Source/Templates/system_stm32wbaxx.c \
             $(WBA_DEV)/Source/Templates/gcc/startup_stm32wba55xx.s \
             $(addprefix $(WBA_HAL)/Src/stm32wbaxx_,hal.c hal_cortex.c hal_rcc.c hal_rcc_ex.c \
               hal_pwr.c hal_pwr_ex.c hal_flash.c hal_flash_ex.c hal_gpio.c ll_rcc.c) \
             $(addprefix $(WPAN)/link_layer/ll_sys/src/,ll_sys_cs.c ll_sys_dp_slp.c ll_sys_intf.c \
               ll_sys_startup.c) \
             $(addprefix $(WBA_EX)/STM32_WPAN/Target/,bleplat.c host_stack_if.c linklayer_plat.c \
               ll_sys_if.c power_table.c)

LIBS := $(WPAN)/link_layer/ll_cmd_lib/lib/LinkLayer_BLE_Peripheral_Only_V3_0.a \
        $(WPAN)/ble/stack/lib/stm32wba_ble_stack_full.a

CPU    := -mcpu=cortex-m33 -mthumb -mfpu=fpv5-sp-d16 -mfloat-abi=hard -mfp16-format=ieee
DEFS   := -DSTM32WBA55xx -DUSE_HAL_DRIVER -DUSE_FULL_LL_DRIVER -DBLE_LL -DPHY_40nm_3_00_a \
          -DBLE_STACK_PERIPHERAL_ONLY
# board/ first: its skel_ble.h stands in for the example's application header (app_conf.h is the
# example's own; ble.c overrides its MBLOCK margin).
INCS   := -I$(APP_DIR) -Iboard -I$(UCLOG_DIR) -I$(UCCOMM_DIR) \
          -I$(WBA_EX)/Core/Inc -I$(WBA_EX)/STM32_WPAN/App -I$(WBA_EX)/STM32_WPAN/Target \
          -I$(WBA_EX)/System/Modules \
          -I$(STM32CUBE_WBA)/Drivers/CMSIS/Include -I$(WBA_DEV)/Include -I$(WBA_HAL)/Inc \
          -I$(WPAN) -I$(WPAN)/ble/stack/include -I$(WPAN)/ble/stack/include/auto \
          -I$(WPAN)/link_layer/ll_cmd_lib/config/ble_peripheral_only \
          -I$(WPAN)/link_layer/ll_cmd_lib/inc -I$(WPAN)/link_layer/ll_cmd_lib/inc/_40nm_reg_files \
          -I$(WPAN)/link_layer/ll_cmd_lib/inc/ot_inc -I$(WPAN)/link_layer/ll_cmd_lib/porting \
          -I$(WPAN)/link_layer/ll_cmd_lib/src/shrd_utils/inc -I$(WPAN)/link_layer/ll_sys/inc
CFLAGS := $(CPU) -std=gnu11 -Os -g3 -Wall -Wextra -Wno-override-init \
          -Wno-unused-parameter -ffunction-sections -fdata-sections \
          -fno-eliminate-unused-debug-types -ffile-prefix-map=$(APP_DIR)/=src/ \
          $(DEFS) $(INCS) -MMD -MP
# ST's files: their warnings are not ours.
CUBE_CFLAGS := $(filter-out -Wall -Wextra -Wno-override-init -Wno-unused-parameter,$(CFLAGS)) -w
LDFLAGS := $(CPU) -Tboard/stm32wba55cg.ld -L$(UCLOG_DIR) -Wl,--gc-sections -Wl,-Map=$(BUILD)/app.map \
           --specs=nano.specs --specs=nosys.specs -u _printf_float -Wl,--no-warn-rwx-segments \
           -Wl,--no-warn-execstack
LDLIBS := -Wl,--start-group $(LIBS) -lm -Wl,--end-group

OBJS := $(patsubst %.c,$(BUILD)/obj/%.o,$(subst ../,,$(SRCS))) \
        $(patsubst %,$(BUILD)/obj/cube/%.o,$(basename $(notdir $(CUBE_SRCS))))

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

$(BUILD)/obj/uclog/%.o: $(UCLOG_DIR)/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/obj/uccomm/%.o: $(UCCOMM_DIR)/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

vpath %.c $(sort $(dir $(filter %.c,$(CUBE_SRCS))))
vpath %.s $(sort $(dir $(filter %.s,$(CUBE_SRCS))))

$(BUILD)/obj/cube/%.o: %.c
	@mkdir -p $(dir $@)
	$(CC) $(CUBE_CFLAGS) -c $< -o $@

$(BUILD)/obj/cube/%.o: %.s
	@mkdir -p $(dir $@)
	$(CC) $(CPU) -c $< -o $@

# 1. link (the .apphash field is zero)
# 2. app_hash = SHA-512 of that image; write it into .apphash
# 3. logdata from the ELF; embed it as .logdata_cbor; save <app_hash>.logdata too
$(BUILD)/app.elf: $(OBJS) board/stm32wba55cg.ld $(UCLOG_DIR)/log.ld $(UCLOG_DIR)/tools/logdata.py \
             $(UCLOG_DIR)/tools/apphash.py
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
BOARD_PROBE ?= STM32WBA55G-DK1
STLINK_SN ?= $(shell $(PROG) -l st-link 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | \
               awk '/ST-LINK SN/ {sn=$$NF} /Board Name/ && $$NF=="$(BOARD_PROBE)" {print sn; exit}')

flash: $(BUILD)/app.bin
	@test -n "$(STLINK_SN)" || { echo "no $(BOARD_PROBE) ST-LINK found (set STLINK_SN)"; exit 1; }
	$(PROG) -c port=SWD mode=UR sn=$(STLINK_SN) -w $(BUILD)/app.bin 0x08000000 -v -rst

clean:
	rm -rf $(BUILD)

.PHONY: all flash clean
-include $(OBJS:.o=.d)
