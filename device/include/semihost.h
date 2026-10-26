// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Semihost client: file and stdio requests to the host on port 63 (spec/protocol.md §5.5), for
// any C library. syscalls.c builds newlib's syscalls on it (bare metal); ports/zephyr/semihost_fs.c
// builds stdout/stdin hooks and a Zephyr file system on it.
//
// Each call returns -1 and sets errno on failure (newlib errno numbers, which picolibc and Zephyr
// share). Calls that need a reply fail at once with EIO when no host is connected: otherwise the
// request would wait in the TX buffer and reach a later host as a stale request. Threads take
// turns (one request is outstanding at a time).

#pragma once

#include <stddef.h>
#include <stdint.h>

#include "log.h"

#ifdef __cplusplus
extern "C" {
#endif

#if !defined(SH_BUF_SIZE) && defined(CONFIG_UC_SYSCALLS_BUF_SIZE)
#define SH_BUF_SIZE CONFIG_UC_SYSCALLS_BUF_SIZE
#endif
#if !defined(SH_BUF_SIZE)
#define SH_BUF_SIZE LOG_MAX_PACKET_SIZE // largest semihost request or reply
#endif

#define UCLOG_SH_PORT 63
#define UCLOG_SH_READ_MAX (SH_BUF_SIZE - 8)  // most bytes one uclog_sh_read() asks for
#define UCLOG_SH_WRITE_MAX (SH_BUF_SIZE - 5) // most bytes one uclog_sh_write() sends

// OPEN flags on the wire: newlib's values, whatever the device's C library uses.
#define UCLOG_SH_O_RDONLY 0
#define UCLOG_SH_O_WRONLY 1
#define UCLOG_SH_O_RDWR 2
#define UCLOG_SH_O_APPEND 0x8
#define UCLOG_SH_O_CREAT 0x200
#define UCLOG_SH_O_TRUNC 0x400
#define UCLOG_SH_O_EXCL 0x800

typedef struct {
  uint32_t mode, ino, dev, nlink, uid, gid, size, atime, mtime, ctime;
} uclog_sh_stat_t;

int uclog_sh_open(const char *path, int flags);         // the host fid
int uclog_sh_close(int fid);
int uclog_sh_read(int fid, void *buf, size_t n);        // at most UCLOG_SH_READ_MAX bytes per call
int uclog_sh_write(int fid, const void *buf, size_t n); // no reply; at most UCLOG_SH_WRITE_MAX
int uclog_sh_fstat(int fid, uclog_sh_stat_t *st);
int uclog_sh_mkdir(const char *path, int mode);
int uclog_sh_isatty(int fid);                           // 1 or 0 (0 with errno set on failure)
int uclog_sh_timeofday(uint64_t *sec, uint32_t *usec);  // the host's wall clock (UTC)

#ifdef __cplusplus
}
#endif
