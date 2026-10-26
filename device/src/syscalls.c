// © 2024 Unit Circle Inc.
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
#include "semihost.h"

// Newlib syscalls over the ucLog semihost port (63, spec/protocol.md §5.5), for bare-metal newlib
// (link this file instead of nosys). From the Unit Circle Zephyr firmware; changes since are
// marked "uclog-clientv2". The requests themselves are in semihost.c. newlib's O_* and errno
// values are the wire's.

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

int __attribute__((weak)) _read (int fd, void * ptr, size_t len) {
  return uclog_sh_read(fd, ptr, len);
}

off_t _lseek (int fd, off_t ptr, int dir) {
  LOG_ERROR("_lseek(%d, %ld, %d)", fd, ptr, dir);
  errno = ENOSYS;
  return -1;
}

// No reply: every write counts as a success, at most UCLOG_SH_WRITE_MAX bytes (a short count).
int __attribute__((weak)) _write (int fd, const void * ptr, size_t len) {
  return uclog_sh_write(fd, ptr, len);
}

int __attribute__((weak)) mkdir (const char * path, mode_t mode) {
  return uclog_sh_mkdir(path, (int) mode);
}

int __attribute__((weak)) _open (const char * path, int flags, ...) {
  return uclog_sh_open(path, flags);
}

int __attribute__((weak)) _close (int fd) {
  return uclog_sh_close(fd);
}

pid_t __attribute__((weak)) _getpid (void) {
  LOG_ERROR("_getpid()");
  return (pid_t)1;
}

int __attribute__((weak)) _fstat (int fd, struct stat * st) {
  uclog_sh_stat_t s;
  int r = uclog_sh_fstat(fd, &s);
  if (r == -1 && errno == EIO) return -1;
  int e = errno;
  st->st_mode = s.mode;
  st->st_ino = s.ino;
  st->st_dev = s.dev;
  st->st_nlink = s.nlink;
  st->st_uid = s.uid;
  st->st_gid = s.gid;
  st->st_size = s.size;
  st->st_atime = s.atime;
  st->st_mtime = s.mtime;
  st->st_ctime = s.ctime;
  errno = e;
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
  uint64_t sec;
  uint32_t usec;
  if (uclog_sh_timeofday(&sec, &usec) != 0) return -1;
  if (tp) {
    tp->tv_sec = sec;
    tp->tv_usec = usec;
  }
  if (tzvp) {
    struct timezone *tzp = tzvp;
    tzp->tz_minuteswest = 0;
    tzp->tz_dsttime = 0;
  }
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
  return uclog_sh_isatty(fd);
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
