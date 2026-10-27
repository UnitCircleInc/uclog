# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
#
# The ucLog device library for Makefile projects (bare metal). Before including, set UCLOG_DIR to
# this directory (default: where this file is), and optionally PYTHON, OBJCOPY, OBJDUMP and
# UCLOG_CACHE. Then:
#
#   SRCS    += $(UCLOG_SRCS) $(UCLOG_SYSCALLS_SRCS)  # the library, ucComm; newlib semihost syscalls
#   CFLAGS  += $(UCLOG_CFLAGS)                       # -I for log.h and ucComm (Arm GCC: and
#                                                    # -mfp16-format=ieee, for ucComm's cbor.h)
#   LDFLAGS += $(UCLOG_LDFLAGS)                      # -L so the linker script can INCLUDE log.ld
#
# and after linking ELF (with the .apphash field still zero):
#
#   $(call uclog_post_link,linked.elf,out.elf,build/dir)
#
# which writes the app hash into .apphash, builds the logdata from the ELF and embeds it as
# .logdata_cbor, and leaves build/dir/app.logdata, app.hash, app.hash.txt (hex) and
# <app_hash>.logdata, plus a copy in the hosts' cache (UCLOG_CACHE).

UCLOG_DIR ?= $(patsubst %/,%,$(dir $(lastword $(MAKEFILE_LIST))))

PYTHON  ?= python3
OBJCOPY ?= arm-none-eabi-objcopy
OBJDUMP ?= arm-none-eabi-objdump
# The hosts find logdata here by the device's app_hash (the same rule as uclog and the Python
# library: $UCLOG_CACHE_DIR, else $XDG_CACHE_HOME/uclog, else ~/.cache/uclog).
UCLOG_CACHE ?= $(or $(UCLOG_CACHE_DIR),$(XDG_CACHE_HOME:%=%/uclog),$(HOME)/.cache/uclog)

UCLOG_SRCS := $(UCLOG_DIR)/src/log.c \
              $(UCLOG_DIR)/src/log_plot.c \
              $(UCLOG_DIR)/src/semihost.c \
              $(UCLOG_DIR)/uccomm/cb.c \
              $(UCLOG_DIR)/uccomm/cobs.c \
              $(UCLOG_DIR)/uccomm/cbor.c \
              $(UCLOG_DIR)/uccomm/crc32c.c
UCLOG_SYSCALLS_SRCS := $(UCLOG_DIR)/src/syscalls.c
UCLOG_CFLAGS := -I$(UCLOG_DIR)/include -I$(UCLOG_DIR)/uccomm
UCLOG_LDFLAGS := -L$(UCLOG_DIR)/ports/baremetal
UCLOG_TOOLS := $(UCLOG_DIR)/tools/logdata.py $(UCLOG_DIR)/tools/hash.py $(UCLOG_DIR)/ports/baremetal/log.ld

# $(1) linked ELF (.apphash zero), $(2) output ELF, $(3) build directory
define uclog_post_link
	$(OBJCOPY) -O binary $(1) $(3)/app.unhashed.bin
	$(PYTHON) $(UCLOG_DIR)/tools/hash.py -f -i $(3)/app.unhashed.bin -o $(3)/app.hash > $(3)/app.hash.txt
	$(OBJCOPY) --update-section .apphash=$(3)/app.hash $(1)
	$(PYTHON) $(UCLOG_DIR)/tools/logdata.py --objdump $(OBJDUMP) $(1) -o $(3)/app.logdata
	$(OBJCOPY) --add-section .logdata_cbor=$(3)/app.logdata --set-section-flags .logdata_cbor=noload,readonly $(1) $(2)
	cp $(3)/app.logdata $(3)/$$(cat $(3)/app.hash.txt).logdata
	@mkdir -p $(UCLOG_CACHE) && cp $(3)/app.logdata $(UCLOG_CACHE)/$$(cat $(3)/app.hash.txt).logdata
endef
