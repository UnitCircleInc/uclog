# A minimal ucLog app on the NUCLEO-L452RE-P (bare metal, Makefile; CMakeLists.txt builds the same).
#
#   make            build/example.elf (logdata embedded), example.bin; <app_hash>.logdata goes to
#                   the hosts' cache, so they decode this build's logs without being told
#   make flash      program it over the board's ST-LINK (STM32_Programmer_CLI)
#   make clean
#
# Needs arm-none-eabi-gcc (on PATH, or from STM32CubeCLT: CUBECLT) and Python 3, and ST's CMSIS
# files for the STM32L4 (STM32CUBE_L4: an STM32CubeL4 package from STM32CubeMX, or a checkout of
# github.com/STMicroelectronics/STM32CubeL4 with its submodules).

CUBECLT ?= /opt/ST/STM32CubeCLT_1.22.0
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

STM32CUBE_L4 ?= $(HOME)/STM32Cube/Repository/STM32Cube_FW_L4_V1.18.2
CMSIS_DEV := $(STM32CUBE_L4)/Drivers/CMSIS/Device/ST/STM32L4xx

# 1. The library: its sources and flags (UCLOG_SRCS, UCLOG_CFLAGS, UCLOG_LDFLAGS, uclog_post_link).
#    UCLOG_SYSCALLS_SRCS: newlib's system calls over the link (semihost), so stdout, stdin and
#    files reach the host; without it, link newlib's stubs (nosys) instead.
UCLOG_DIR := ../..
include $(UCLOG_DIR)/uclog.mk

BUILD := build
SRCS  := main.c port.c $(UCLOG_SRCS) $(UCLOG_SYSCALLS_SRCS) \
         $(CMSIS_DEV)/Source/Templates/system_stm32l4xx.c \
         $(CMSIS_DEV)/Source/Templates/gcc/startup_stm32l452xx.s

# -g: logdata.py reads the format strings' argument types (and enums) from the debug info.
# -ffile-prefix-map: the file names the logs show (uclog/src/log.c rather than ../../src/log.c).
CPU    := -mcpu=cortex-m4 -mthumb -mfpu=fpv4-sp-d16 -mfloat-abi=hard
CFLAGS := $(CPU) -std=gnu11 -Os -g -Wall -Wextra -Wno-unused-parameter -ffunction-sections \
          -fdata-sections -ffile-prefix-map=$(UCLOG_DIR)/=uclog/ -DSTM32L452xx \
          -I$(STM32CUBE_L4)/Drivers/CMSIS/Include -I$(CMSIS_DEV)/Include $(UCLOG_CFLAGS) -mfp16-format=ieee
LDFLAGS := $(CPU) -Tstm32l452re.ld $(UCLOG_LDFLAGS) -Wl,--gc-sections --specs=nano.specs \
           --specs=nosys.specs -Wl,--no-warn-rwx-segments

OBJS := $(addprefix $(BUILD)/obj/,$(addsuffix .o,$(notdir $(basename $(SRCS)))))
vpath %.c $(sort $(dir $(filter %.c,$(SRCS))))
vpath %.s $(dir $(filter %.s,$(SRCS)))

all: $(BUILD)/example.bin

$(BUILD)/obj/%.o: %.c | $(BUILD)/obj
	$(CC) $(CFLAGS) -MMD -MP -c $< -o $@

$(BUILD)/obj/%.o: %.s | $(BUILD)/obj
	$(CC) $(CPU) -x assembler-with-cpp -c $< -o $@

$(BUILD)/obj:
	mkdir -p $@

# 2. Link with the app hash field zeroed, then uclog_post_link: the app hash (SHA-512 of the
#    image) goes into .apphash, and the logdata (format strings and types, from the ELF) is
#    embedded as .logdata_cbor and copied to the hosts' cache.
$(BUILD)/example.elf: $(OBJS) stm32l452re.ld $(UCLOG_TOOLS)
	$(CC) $(LDFLAGS) $(OBJS) -o $@.tmp
	$(call uclog_post_link,$@.tmp,$@,$(BUILD))
	@rm -f $@.tmp
	$(SIZE) $@

$(BUILD)/example.bin: $(BUILD)/example.elf
	$(OBJCOPY) -O binary $< $@

flash: $(BUILD)/example.bin
	$(PROG) -c port=SWD mode=UR $(if $(STLINK_SN),sn=$(STLINK_SN)) -w $< 0x08000000 -v -rst

clean:
	rm -rf $(BUILD)

.PHONY: all flash clean
-include $(OBJS:.o=.d)
