// Semihost (port 63) selftest: newlib I/O through syscalls.c, plus the time ops and error replies
// as raw requests.
#pragma once

#include <stddef.h>
#include <stdint.h>

// Raw call: request `op ‖ args`, reply copied to `reply`. Returns the reply length (0 when there
// is no host or no reply).
size_t sh_call(uint8_t op, const void *args, size_t n, void *reply, size_t cap);

void sh_selftest(void);
