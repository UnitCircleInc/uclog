// © 2024 Unit Circle Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#include <_ansi.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <sys/fcntl.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <sys/time.h>
#include <sys/times.h>
#include <errno.h>
#include <reent.h>
#include <unistd.h>
#include <sys/wait.h>
#include <stdbool.h>

#include "log.h"

// Newlib syscalls over the ucLog semihost port (63), from the Unit Circle Zephyr firmware.
// Changes for this bare-metal test firmware are marked "uclog-clientv2".

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

// All the possible "syscalls"
int	_system		(const char *);
int	_rename		(const char *, const char *);
int	_isatty		(int);
clock_t _times		(struct tms *);
int	_gettimeofday	(struct timeval *, void *);
int	_unlink		(const char *);
int	_link		(const char *, const char *);
int	_stat		(const char *, struct stat *);
int	_fstat		(int, struct stat *);
void *	_sbrk		(ptrdiff_t);
pid_t	_getpid		(void);
int	_close		(int);
clock_t	_clock		(void);
int	_open		(const char *, int, ...);
int	_write		(int, const void *, size_t);
_off_t	_lseek		(int, _off_t, int);
int	_read		(int, void *, size_t);
int	mkdir		(const char *, mode_t mode);

static uint8_t txrx[LOG_MAX_PACKET_SIZE];

// uclog-clientv2: a request that needs a reply fails at once when no host is connected (otherwise
// it would wait in the TX buffer and reach a later host as a stale request).
static bool no_host(void) {
  bool host = false;
  log_is_host_ready(&host);
  if (!host) errno = EIO;
  return !host;
}

int __attribute__((weak)) _read (int fd, void * ptr, size_t len) {
  if (no_host()) return -1;
  size_t n = len;
  if (n > sizeof(txrx) - 8) n = sizeof(txrx) - 8;
  txrx[0] = SH_READ;
  txrx[1] = (fd >>  0) & 0xff;
  txrx[2] = (fd >>  8) & 0xff;
  txrx[3] = (fd >> 16) & 0xff;
  txrx[4] = (fd >> 24) & 0xff;
  // uclog-clientv2: request the capped length n (was len).
  txrx[5] = (n >>  0) & 0xff;
  txrx[6] = (n >>  8) & 0xff;
  txrx[7] = (n >> 16) & 0xff;
  txrx[8] = (n >> 24) & 0xff;

  log_tx(63, txrx, 9);
  size_t rxn = log_rx(63, txrx, sizeof(txrx));
  if (rxn < 8) {
    LOG_ERROR("_read - response incomplete");
    errno = EIO;
    return -1;
  }
  int r = (txrx[0] << 0) | (txrx[1] << 8) | (txrx[2] << 16) | (txrx[3] << 24);
  errno = (txrx[4] << 0) | (txrx[5] << 8) | (txrx[6] << 16) | (txrx[7] << 24);
  if (r > 0) {
    if ((size_t) r > rxn - 8) r = rxn - 8; // uclog-clientv2: never copy past the reply
    memmove(ptr, txrx+8, r);
  }
  //LOG_INFO("%d = _read(%d, %p, %zu) (errno: %d)", r, fd, ptr, len, errno);
  if (r >= 0) errno = 0; // uclog-clientv2: keep errno on failure (was always cleared)
  return r;
}

off_t _lseek (int fd, off_t ptr, int dir) {
  LOG_ERROR("_lseek(%d, %ld, %d)", fd, ptr, dir);
  errno = ENOSYS;
  return -1;
}

//#define WRITE_EXPECT_RESPONSE

int __attribute__((weak)) _write (int fd, const void * ptr, size_t len) {
  size_t n = len;
  if (n > sizeof(txrx) - 5) n = sizeof(txrx) - 5;

  txrx[0] = SH_WRITE;
  txrx[1] = (fd >>  0) & 0xff;
  txrx[2] = (fd >>  8) & 0xff;
  txrx[3] = (fd >> 16) & 0xff;
#if defined(WRITE_EXPECT_RESPONSE)
  txrx[4] = (fd >> 24) & 0xff;
#else
  // MSB set indicates that write does not expect a response
  // open returns values in range 0-0x7ffffff so can use MSB
  // this allows for backward compatibility or in the future
  // some sort of runtime selection.
  txrx[4] = ((fd >> 24) & 0xff) | 0x80;
#endif

  for (size_t i = 0; i < n; i++) {
    txrx[i + 5] = (uint8_t) i;
  }
  memmove(txrx+5, ptr, n);

  //LOG_INFO("log_tx_avail() %zu", log_tx_avail());
  //size_t wait = 0;
  // uclog-clientv2: no k_sleep; the TX DMA drains the buffer. Give up if there is no host.
  bool host = true;
  while (log_tx_avail() <  n + 5 && log_is_host_ready(&host) == 0 && host) {
    //wait += 1;
  }
  //LOG_INFO("log_tx_avail() wait %zu", wait);
  log_tx(63, txrx, n + 5);
#if defined(WRITE_EXPECT_RESPONSE)
  size_t rxn = log_rx(63, txrx, sizeof(txrx));
  if (rxn != 8) {
    LOG_ERROR("_write - response incomplete");
    errno = EIO;
    return -1;
  }
  int r = (txrx[0] << 0) | (txrx[1] << 8) | (txrx[2] << 16) | (txrx[3] << 24);
  errno = (txrx[4] << 0) | (txrx[5] << 8) | (txrx[6] << 16) | (txrx[7] << 24);
  //LOG_INFO("%d = _write(%d, %p, %zu) (errno: %d)", r, fd, ptr, len, errno);
  errno = 0;
  return r;
#else
  // To improve throughput pretend all writes are a success
  // that we we don't need to wait for the response 
  errno = 0;
  return n;
#endif
}


int __attribute__((weak)) mkdir (const char * path, mode_t mode) {
  if (no_host()) return -1;
  txrx[0] = SH_MKDIR;
  txrx[1] = (mode >>  0) & 0xff;
  txrx[2] = (mode >>  8) & 0xff;
  txrx[3] = (mode >> 16) & 0xff;
  txrx[4] = (mode >> 24) & 0xff;
  size_t n = strlen(path);
  if (n + 5 > sizeof(txrx)) {
    LOG_FATAL("path too long %zu > %zu", n, sizeof(txrx));
  }
  memmove(txrx + 5, path, n);
  log_tx(63, txrx, 5 + n);
  size_t rxn = log_rx(63, txrx, sizeof(txrx));
  if (rxn != 8) {
    LOG_ERROR("_open - response incomplete");
      errno = EIO;
      return -1;
  }
  int r = (txrx[0] << 0) | (txrx[1] << 8) | (txrx[2] << 16) | (txrx[3] << 24);
  errno = (txrx[4] << 0) | (txrx[5] << 8) | (txrx[6] << 16) | (txrx[7] << 24);
  //LOG_INFO("%d = mkdir(%s, 0x%04x, ...) (errno: %d)", r, path, (unsigned int) mode, errno);
  return r;
}


int __attribute__((weak)) _open (const char * path, int flags, ...) {
  if (no_host()) return -1;
  txrx[0] = SH_OPEN;
  txrx[1] = (flags >>  0) & 0xff;
  txrx[2] = (flags >>  8) & 0xff;
  txrx[3] = (flags >> 16) & 0xff;
  txrx[4] = (flags >> 24) & 0xff;
  size_t n = strlen(path);
  if (n + 5 > sizeof(txrx)) {
    LOG_FATAL("path too long %zu > %zu", n, sizeof(txrx));
  }
  memmove(txrx + 5, path, n);
  log_tx(63, txrx, 5 + n);
  size_t rxn = log_rx(63, txrx, sizeof(txrx));
  if (rxn != 8) {
    LOG_ERROR("_open - response incomplete");
      errno = EIO;
      return -1;
  }
  int r = (txrx[0] << 0) | (txrx[1] << 8) | (txrx[2] << 16) | (txrx[3] << 24);
  errno = (txrx[4] << 0) | (txrx[5] << 8) | (txrx[6] << 16) | (txrx[7] << 24);
  //LOG_INFO("%d = _open(%s, 0x%04x, ...) (errno: %d)", r, path, flags, errno);
  return r;
}

int __attribute__((weak)) _close (int fd) {
  if (no_host()) return -1;
  txrx[0] = SH_CLOSE;
  txrx[1] = (fd >>  0) & 0xff;
  txrx[2] = (fd >>  8) & 0xff;
  txrx[3] = (fd >> 16) & 0xff;
  txrx[4] = (fd >> 24) & 0xff;
  log_tx(63, txrx, 5);
  size_t rxn = log_rx(63, txrx, sizeof(txrx));
  if (rxn != 8) {
    LOG_ERROR("_close - response incomplete");
      errno = EIO;
      return -1;
  }
  int r = (txrx[0] << 0) | (txrx[1] << 8) | (txrx[2] << 16) | (txrx[3] << 24);
  errno = (txrx[4] << 0) | (txrx[5] << 8) | (txrx[6] << 16) | (txrx[7] << 24);
  //LOG_INFO("%d = _close(%d) (errno: %d)", r, fd, errno);
  return r;
}

pid_t __attribute__((weak)) _getpid (void) {
  LOG_ERROR("_getpid()");
  return (pid_t)1;
}

#if 0
unsigned __heap_limit = 0xcafedead;

void * __attribute__((weak)) _sbrk (ptrdiff_t incr) {
  extern char   end[];
  static char * heap_end;
  char *        prev_heap_end;
  char *        stack_ptr;

  //LOG_INFO("_sbrk(%d)", incr);

  if (heap_end == NULL)
    heap_end = end;

  prev_heap_end = heap_end;

  __asm volatile ( \
      "mov %0, sp \n" \
      : "=r" (stack_ptr));

  if ((heap_end + incr > stack_ptr)
      /* Honour heap limit if it's valid.  */
      || (__heap_limit != 0xcafedead && heap_end + incr > (char *)__heap_limit))
    {
      errno = ENOMEM;
      return (void *) -1;
    }

  heap_end += incr;

  return (void *) prev_heap_end;
}
#endif

int __attribute__((weak)) _fstat (int fd, struct stat * st) {
  if (no_host()) return -1;
  txrx[0] = SH_FSTAT;
  txrx[1] = (fd >>  0) & 0xff;
  txrx[2] = (fd >>  8) & 0xff;
  txrx[3] = (fd >> 16) & 0xff;
  txrx[4] = (fd >> 24) & 0xff;
  log_tx(63, txrx, 5);
  size_t rxn = log_rx(63, txrx, sizeof(txrx));
  if (rxn != 8 + 10 * 4) {
    LOG_ERROR("_fstat - response incomplete");
    errno = EIO;
    return -1;
  }
  int r = (txrx[0] << 0) | (txrx[1] << 8) | (txrx[2] << 16) | (txrx[3] << 24);
  errno = (txrx[4] << 0) | (txrx[5] << 8) | (txrx[6] << 16) | (txrx[7] << 24);
  st->st_mode = (txrx[ 8] << 0) | (txrx[ 9] << 8) | (txrx[10] << 16) | (txrx[11] << 24);
  st->st_ino  = (txrx[12] << 0) | (txrx[13] << 8) | (txrx[14] << 16) | (txrx[15] << 24);
  st->st_dev  = (txrx[16] << 0) | (txrx[17] << 8) | (txrx[18] << 16) | (txrx[19] << 24);
  st->st_nlink= (txrx[20] << 0) | (txrx[21] << 8) | (txrx[22] << 16) | (txrx[23] << 24);
  st->st_uid  = (txrx[24] << 0) | (txrx[25] << 8) | (txrx[26] << 16) | (txrx[27] << 24); // uclog-clientv2: was [27] twice
  st->st_gid  = (txrx[28] << 0) | (txrx[29] << 8) | (txrx[30] << 16) | (txrx[31] << 24);
  st->st_size = (txrx[32] << 0) | (txrx[33] << 8) | (txrx[34] << 16) | (txrx[35] << 24);
  st->st_atime= (txrx[36] << 0) | (txrx[37] << 8) | (txrx[38] << 16) | (txrx[39] << 24);
  st->st_mtime= (txrx[40] << 0) | (txrx[41] << 8) | (txrx[42] << 16) | (txrx[43] << 24);
  st->st_ctime= (txrx[44] << 0) | (txrx[45] << 8) | (txrx[46] << 16) | (txrx[47] << 24);
  //LOG_INFO("%d = _fstat(%d) (errno: %d)", r, fd, errno);
  return r;
}

int __attribute__((weak)) _stat (const char *fname, struct stat *st) {
  LOG_ERROR("_stat(%s, %p)", fname, st);
  errno = ENOSYS;
  return -1;
}

int __attribute__((weak)) _link (const char *path1, const char *path2) {
  LOG_ERROR("_link(%s, %s)", path1, path2);
  errno = ENOSYS;
  return -1;
}

int _unlink (const char *path) {
  LOG_ERROR("_unlink(%s)", path);
  errno = ENOSYS;
  return -1;
}

int _gettimeofday (struct timeval * tp, void * tzvp) {
  if (no_host()) return -1;
  txrx[0] = SH_TIMEOFDAY;
  log_tx(63, txrx, 1);
  size_t rxn = log_rx(63, txrx, sizeof(txrx));
  if (rxn != 12) {
    LOG_ERROR("_gettimeofday - response incomplete");
      errno = EIO;
      return -1;
  }

  // Returned time is UTC
  uint64_t tv_sec = txrx[7];
  tv_sec = (tv_sec << 8) | txrx[6];
  tv_sec = (tv_sec << 8) | txrx[5];
  tv_sec = (tv_sec << 8) | txrx[4];
  tv_sec = (tv_sec << 8) | txrx[3];
  tv_sec = (tv_sec << 8) | txrx[2];
  tv_sec = (tv_sec << 8) | txrx[1];
  tv_sec = (tv_sec << 8) | txrx[0];
  uint32_t tv_usec = (txrx[8] << 0) | (txrx[9] << 8) | (txrx[10] << 16) | (txrx[11] << 24);

  if (tp) {
    tp->tv_sec = tv_sec;
    tp->tv_usec = tv_usec;
  }

  if (tzvp) {
    struct timezone *tzp = tzvp;
    tzp->tz_minuteswest = 0;
    tzp->tz_dsttime = 0;
  }
  errno = 0;
  return 0;
}

/* Return a clock that ticks at 100Hz.  */
clock_t _clock (void) {
  LOG_ERROR("_clock()");
  clock_t timeval = {0};
  return timeval;
}

/* Return a clock that ticks at 100Hz.  */
clock_t _times (struct tms * tp) {
  LOG_ERROR("_times(%p)", tp);
  clock_t timeval = _clock();

  if (tp)
    {
      tp->tms_utime  = timeval;	/* user time */
      tp->tms_stime  = 0;	/* system time */
      tp->tms_cutime = 0;	/* user time, children */
      tp->tms_cstime = 0;	/* system time, children */
    }

  return timeval;
};

int _isatty (int fd) {
  if (no_host()) return -1;
  txrx[0] = SH_ISATTY;
  txrx[1] = (fd >>  0) & 0xff;
  txrx[2] = (fd >>  8) & 0xff;
  txrx[3] = (fd >> 16) & 0xff;
  txrx[4] = (fd >> 24) & 0xff;
  log_tx(63, txrx, 5);
  size_t rxn = log_rx(63, txrx, sizeof(txrx));
  if (rxn != 8) {
    LOG_ERROR("_isatty - response incomplete");
    errno = EIO;
    return -1;
  }
  int r = (txrx[0] << 0) | (txrx[1] << 8) | (txrx[2] << 16) | (txrx[3] << 24);
  errno = (txrx[4] << 0) | (txrx[5] << 8) | (txrx[6] << 16) | (txrx[7] << 24);
  //LOG_INFO("%d = _isatty(%d) (errno: %d)", r, fd, errno);
  return r;
}

int _system (const char *s) {
  LOG_ERROR("_system(%s)", s);
  errno = EPERM;
  return -1;
}

int _rename (const char * oldpath, const char * newpath) {
  LOG_ERROR("_rename(%s, %s)", oldpath, newpath);
  errno = ENOSYS;
  return -1;
}

