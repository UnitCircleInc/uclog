// Port-63 semihost client (spec/protocol.md §5.5): newlib syscalls (printf, fopen, ...) and the
// time ops are served by the host. Without a host, stdout is discarded and the rest fail with EIO.
#pragma once

#include <stddef.h>
#include <stdint.h>

// Raw call: request `op ‖ args`, reply copied to `reply`. Returns the reply length, or -1 when
// there is no host or no reply within the timeout.
int sh_call(uint8_t op, const void *args, size_t n, void *reply, size_t cap);

uint32_t sh_clock(void);     // centiseconds (wraps)
int64_t sh_time(void);       // seconds since 1970
uint64_t sh_elapsed(void);   // host monotonic ticks
uint64_t sh_tickfreq(void);  // ticks per second

// Exercise every op, logging the results.
void sh_selftest(void);
