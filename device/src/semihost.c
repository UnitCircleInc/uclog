// © 2024 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Semihost client (semihost.h). From the newlib syscalls of the Unit Circle Zephyr firmware;
// changes since are marked "uclog-clientv2".

#include "semihost.h"

#include <errno.h>
#include <string.h>

enum semihost_cmds_e {
  SH_SYSTEM = 0,
  SH_RENAME = 1,
  SH_ISATTY = 2,
  SH_UNLINK = 3,
  SH_LINK = 4,
  SH_STAT = 5,
  SH_FSTAT = 6,
  SH_CLOSE = 7,
  SH_OPEN = 8,
  SH_WRITE = 9,
  SH_LSEEK = 10,
  SH_READ = 11,
  SH_MKDIR = 12,
  SH_TIMEOFDAY = 13,
};

static uint8_t txrx[SH_BUF_SIZE];

static void put32(uint8_t *b, uint32_t v) {
  b[0] = (uint8_t)v;
  b[1] = (uint8_t)(v >> 8);
  b[2] = (uint8_t)(v >> 16);
  b[3] = (uint8_t)(v >> 24);
}

static uint32_t get32(const uint8_t *b) {
  return (uint32_t)b[0] | ((uint32_t)b[1] << 8) | ((uint32_t)b[2] << 16) | ((uint32_t)b[3] << 24);
}

// uclog-clientv2: the reply length, at most sizeof(txrx). log_rx returns the payload length, which
// can exceed what it copied: without the clamp a long reply made the callers read past txrx.
static size_t sh_rx(void) {
  size_t n = log_rx(UCLOG_SH_PORT, txrx, sizeof(txrx));
  return n < sizeof(txrx) ? n : sizeof(txrx);
}

// uclog-clientv2: a request that needs a reply fails at once when no host is connected (otherwise
// it would wait in the TX buffer and reach a later host as a stale request).
static bool no_host(void) {
  bool host = false;
  log_is_host_ready(&host);
  if (!host) errno = EIO;
  return !host;
}

// `<i32 r, i32 errno>`: r, with errno set from the reply.
static int r_errno(void) {
  errno = (int)get32(txrx + 4);
  return (int)get32(txrx);
}

// The request in txrx[0, n), then its `<i32 r, i32 errno>` reply (of `want` bytes).
static int call(size_t n, size_t want, const char *what) {
  log_tx(UCLOG_SH_PORT, txrx, n);
  if (sh_rx() != want) {
    LOG_ERROR("%s - response incomplete", what);
    errno = EIO;
    return -1;
  }
  return r_errno();
}

// Requests from several threads take turns (log_port.h; nothing on a single-threaded port).
#define BEGIN() log_port_rx_begin()
#define END(r_)                                                                                \
  do {                                                                                         \
    int end_r_ = (r_);                                                                         \
    log_port_rx_end();                                                                         \
    return end_r_;                                                                             \
  } while (0)

static int fid_call(uint8_t op, int fid, size_t want, const char *what) {
  txrx[0] = op;
  put32(txrx + 1, (uint32_t)fid);
  return call(5, want, what);
}

static int path_call(uint8_t op, uint32_t arg, const char *path, const char *what) {
  size_t n = strlen(path);
  if (n + 5 > sizeof(txrx)) {
    errno = ENAMETOOLONG;
    return -1;
  }
  txrx[0] = op;
  put32(txrx + 1, arg);
  memcpy(txrx + 5, path, n);
  return call(5 + n, 8, what);
}

int uclog_sh_open(const char *path, int flags) {
  if (no_host()) return -1;
  BEGIN();
  END(path_call(SH_OPEN, (uint32_t)flags, path, "open"));
}

int uclog_sh_mkdir(const char *path, int mode) {
  if (no_host()) return -1;
  BEGIN();
  END(path_call(SH_MKDIR, (uint32_t)mode, path, "mkdir"));
}

int uclog_sh_close(int fid) {
  if (no_host()) return -1;
  BEGIN();
  END(fid_call(SH_CLOSE, fid, 8, "close"));
}

int uclog_sh_isatty(int fid) {
  if (no_host()) return 0;
  BEGIN();
  int r = fid_call(SH_ISATTY, fid, 8, "isatty");
  // uclog-clientv2: failures return 0 with errno set (was -1, which newlib takes as "a tty").
  END(r > 0);
}

int uclog_sh_read(int fid, void *buf, size_t len) {
  if (no_host()) return -1;
  BEGIN();
  size_t n = len < UCLOG_SH_READ_MAX ? len : UCLOG_SH_READ_MAX;
  txrx[0] = SH_READ;
  put32(txrx + 1, (uint32_t)fid);
  put32(txrx + 5, (uint32_t)n); // uclog-clientv2: request the capped length n (was len)
  log_tx(UCLOG_SH_PORT, txrx, 9);
  size_t rxn = sh_rx();
  if (rxn < 8) {
    LOG_ERROR("read - response incomplete");
    errno = EIO;
    END(-1);
  }
  int r = r_errno();
  if (r > 0) {
    // uclog-clientv2: never copy past the reply, or more than was asked for.
    if ((size_t)r > rxn - 8) r = (int)(rxn - 8);
    if ((size_t)r > n) r = (int)n;
    memcpy(buf, txrx + 8, (size_t)r);
  }
  if (r >= 0) errno = 0; // uclog-clientv2: keep errno on failure (was always cleared)
  END(r);
}

int uclog_sh_write(int fid, const void *buf, size_t len) {
  BEGIN();
  size_t n = len < UCLOG_SH_WRITE_MAX ? len : UCLOG_SH_WRITE_MAX;
  txrx[0] = SH_WRITE;
  // Bit 31 set: no reply wanted (fids are 0..0x7fffffff). Waiting for each reply would cost a
  // round trip per write, so every write counts as a success.
  put32(txrx + 1, (uint32_t)fid | 0x80000000u);
  memcpy(txrx + 5, buf, n);
  // uclog-clientv2: no k_sleep; the link drains the buffer (a threaded port lets its log thread
  // run). Give up if there is no host.
  bool host = true;
  while (log_tx_avail() < n + 5 && log_is_host_ready(&host) == 0 && host) log_port_yield();
  log_tx(UCLOG_SH_PORT, txrx, n + 5);
  errno = 0;
  END((int)n);
}

int uclog_sh_fstat(int fid, uclog_sh_stat_t *st) {
  if (no_host()) return -1;
  BEGIN();
  txrx[0] = SH_FSTAT;
  put32(txrx + 1, (uint32_t)fid);
  log_tx(UCLOG_SH_PORT, txrx, 5);
  if (sh_rx() != 8 + 10 * 4) {
    LOG_ERROR("fstat - response incomplete");
    errno = EIO;
    END(-1);
  }
  int r = r_errno();
  {
    const uint8_t *b = txrx + 8;
    st->mode = get32(b);
    st->ino = get32(b + 4);
    st->dev = get32(b + 8);
    st->nlink = get32(b + 12);
    st->uid = get32(b + 16); // uclog-clientv2: was byte 27 twice
    st->gid = get32(b + 20);
    st->size = get32(b + 24);
    st->atime = get32(b + 28);
    st->mtime = get32(b + 32);
    st->ctime = get32(b + 36);
  }
  END(r);
}

int uclog_sh_timeofday(uint64_t *sec, uint32_t *usec) {
  if (no_host()) return -1;
  BEGIN();
  txrx[0] = SH_TIMEOFDAY;
  log_tx(UCLOG_SH_PORT, txrx, 1);
  if (sh_rx() != 12) {
    LOG_ERROR("gettimeofday - response incomplete");
    errno = EIO;
    END(-1);
  }
  *sec = (uint64_t)get32(txrx) | ((uint64_t)get32(txrx + 4) << 32); // UTC
  *usec = get32(txrx + 8);
  errno = 0;
  END(0);
}
