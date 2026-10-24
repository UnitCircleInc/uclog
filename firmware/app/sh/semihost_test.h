// Semihost (port 63) selftest: newlib I/O through syscalls.c, plus the time ops and error replies
// as raw requests.
#pragma once

#include <stddef.h>
#include <stdint.h>

#include "log.h"

// Boards with little RAM set these smaller (the Makefile's -D).
#ifndef SH_BUF_SIZE
#define SH_BUF_SIZE LOG_MAX_PACKET_SIZE // largest semihost request or reply (syscalls.c, selftest)
#endif
#ifndef SH_BIG_FILE
#define SH_BIG_FILE 4000 // the selftest's file written and read back in several calls
#endif

// Raw call: request `op ‖ args`, reply copied to `reply`. Returns the reply length (0 when there
// is no host or no reply).
size_t sh_call(uint8_t op, const void *args, size_t n, void *reply, size_t cap);

void sh_selftest(void);
