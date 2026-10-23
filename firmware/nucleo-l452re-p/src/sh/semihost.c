#include "semihost.h"

#include <errno.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <unistd.h>

#include "log.h"

#define SH_PORT 63
#define SH_TIMEOUT_MS 5000
#define SH_MAX_IO 1024 // bytes per READ/WRITE request

enum sh_op {
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
  SH_CLOCK = 0x10,
  SH_TIME = 0x11,
  SH_ELAPSED = 0x30,
  SH_TICKFREQ = 0x31,
};

static uint8_t req[1 + 8 + SH_MAX_IO];
static uint32_t timeout_ms = SH_TIMEOUT_MS;
static uint8_t rsp[8 + SH_MAX_IO];

static bool host(void) {
  bool ready;
  log_is_host_ready(&ready);
  return ready;
}

int sh_call(uint8_t op, const void *args, size_t n, void *reply, size_t cap) {
  if (!host() || n + 1 > sizeof(req)) return -1;
  req[0] = op;
  memcpy(req + 1, args, n);
  if (!log_tx(SH_PORT, req, n + 1)) return -1;
  size_t r = log_rx(SH_PORT, reply, cap, timeout_ms);
  if (r == LOG_RX_ERROR) {
    LOG_ERROR("semihost op {enum:sh_op}%d: no reply", op);
    return -1;
  }
  return (int)(r < cap ? r : cap);
}

// `<i32 r, i32 errno>` replies: returns r, sets errno.
static int sh_rerr(uint8_t op, const void *args, size_t n, size_t *len) {
  int got = sh_call(op, args, n, rsp, sizeof(rsp));
  if (len) *len = got < 0 ? 0 : (size_t)got;
  if (got < 8) {
    errno = EIO;
    return -1;
  }
  int32_t r, e;
  memcpy(&r, rsp, 4);
  memcpy(&e, rsp + 4, 4);
  if (e != 0) errno = e;
  return r;
}

static int sh_path(uint8_t op, int32_t v, const char *path) {
  size_t pn = strlen(path);
  if (pn > SH_MAX_IO) {
    errno = ENAMETOOLONG;
    return -1;
  }
  uint8_t a[4 + SH_MAX_IO];
  memcpy(a, &v, 4);
  memcpy(a + 4, path, pn);
  return sh_rerr(op, a, 4 + pn, NULL);
}

static uint64_t sh_u64(uint8_t op) {
  uint64_t v = 0;
  if (sh_call(op, NULL, 0, rsp, sizeof(rsp)) >= 8) memcpy(&v, rsp, 8);
  return v;
}

uint32_t sh_clock(void) {
  uint32_t v = 0;
  if (sh_call(SH_CLOCK, NULL, 0, rsp, sizeof(rsp)) >= 4) memcpy(&v, rsp, 4);
  return v;
}

int64_t sh_time(void) {
  return (int64_t)sh_u64(SH_TIME);
}

uint64_t sh_elapsed(void) {
  return sh_u64(SH_ELAPSED);
}

uint64_t sh_tickfreq(void) {
  return sh_u64(SH_TICKFREQ);
}

// ---------------------------------------------------------------------------------------------
// newlib syscalls

// Like the Zephyr firmware's syscalls.c: WRITE with bit 31 set (no reply wanted), reporting
// success at once. Throughput matters more than write errors here.
#define SH_WRITE_NO_REPLY 0x80000000u

int _write(int fd, const char *buf, int n) {
  if (!host()) return n; // nowhere to go
  for (int done = 0; done < n;) {
    int c = n - done > SH_MAX_IO ? SH_MAX_IO : n - done;
    uint32_t fid = (uint32_t)fd | SH_WRITE_NO_REPLY;
    memcpy(req + 1, &fid, 4);
    memcpy(req + 5, buf + done, (size_t)c);
    req[0] = SH_WRITE;
    uint32_t t0 = log_port_ms();
    while (log_tx_avail() < (size_t)c + 5 && log_port_ms() - t0 < SH_TIMEOUT_MS) {
    }
    if (!log_tx(SH_PORT, req, (size_t)c + 5)) {
      errno = EIO;
      return done > 0 ? done : -1;
    }
    done += c;
  }
  return n;
}

int _read(int fd, char *buf, int n) {
  int32_t a[2] = {fd, n > SH_MAX_IO ? SH_MAX_IO : n};
  size_t len;
  // stdin waits for the user to type a line: no timeout (log_rx still gives up if the host goes).
  timeout_ms = fd == 0 ? LOG_RX_FOREVER : SH_TIMEOUT_MS;
  int r = sh_rerr(SH_READ, a, sizeof(a), &len);
  timeout_ms = SH_TIMEOUT_MS;
  if (r <= 0) return r;
  if ((size_t)r > len - 8) r = (int)(len - 8);
  memcpy(buf, rsp + 8, (size_t)r);
  return r;
}

int _open(const char *path, int flags, ...) {
  return sh_path(SH_OPEN, flags, path);
}

int _close(int fd) {
  int32_t a = fd;
  return sh_rerr(SH_CLOSE, &a, 4, NULL);
}

int _isatty(int fd) {
  int32_t a = fd;
  int r = sh_rerr(SH_ISATTY, &a, 4, NULL);
  return r > 0;
}

int _fstat(int fd, struct stat *st) {
  int32_t a = fd;
  size_t len;
  int r = sh_rerr(SH_FSTAT, &a, 4, &len);
  memset(st, 0, sizeof(*st));
  if (r < 0 || len < 48) return -1;
  uint32_t f[10];
  memcpy(f, rsp + 8, sizeof(f));
  st->st_mode = f[0];
  st->st_ino = f[1];
  st->st_dev = f[2];
  st->st_nlink = f[3];
  st->st_uid = f[4];
  st->st_gid = f[5];
  st->st_size = f[6];
  st->st_atime = f[7];
  st->st_mtime = f[8];
  st->st_ctime = f[9];
  return 0;
}

int _lseek(int fd, int off, int whence) {
  int32_t a[3] = {fd, off, whence};
  return sh_rerr(SH_LSEEK, a, sizeof(a), NULL); // hosts reply ENOSYS
}

int mkdir(const char *path, mode_t mode) {
  return sh_path(SH_MKDIR, (int32_t)mode, path);
}

int _gettimeofday(struct timeval *tv, void *tz) {
  (void)tz;
  if (sh_call(SH_TIMEOFDAY, NULL, 0, rsp, sizeof(rsp)) < 12) {
    errno = EIO;
    return -1;
  }
  uint64_t sec;
  uint32_t usec;
  memcpy(&sec, rsp, 8);
  memcpy(&usec, rsp + 8, 4);
  tv->tv_sec = (time_t)sec;
  tv->tv_usec = (suseconds_t)usec;
  return 0;
}

// ---------------------------------------------------------------------------------------------

static void expect_err(const char *what, int r, int want) {
  if (r == -1 && errno == want) {
    LOG_INFO("%s: -1 errno %d as expected", what, errno);
  } else {
    LOG_ERROR("%s: got %d errno %d, want -1 errno %d", what, r, errno, want);
  }
}

void sh_selftest(void) {
  if (!host()) {
    LOG_WARN("semihost selftest: no host");
    return;
  }
  LOG_INFO("semihost selftest start");

  // Time ops.
  struct timeval tv;
  int r = gettimeofday(&tv, NULL);
  LOG_INFO("TIMEOFDAY r=%d sec=%lld usec=%ld", r, (long long)tv.tv_sec, (long)tv.tv_usec);
  int64_t t = sh_time();
  LOG_INFO("SYS_TIME %lld (delta to TIMEOFDAY %lld)", t, t - (long long)tv.tv_sec);
  uint64_t f = sh_tickfreq(), e0 = sh_elapsed();
  uint32_t c0 = sh_clock();
  uint32_t t0 = log_port_ms();
  while (log_port_ms() - t0 < 200) log_poll();
  uint64_t e1 = sh_elapsed();
  uint32_t c1 = sh_clock();
  LOG_INFO("SYS_TICKFREQ %llu, SYS_ELAPSED delta %llu ticks over 200 ms", f, e1 - e0);
  LOG_INFO("SYS_CLOCK %lu -> %lu (delta %lu cs)", (unsigned long)c0, (unsigned long)c1,
           (unsigned long)(c1 - c0));
  if (f != 0) {
    double ms = (double)(e1 - e0) * 1000.0 / (double)f;
    LOG_INFO("host measured %.1f ms for a 200 ms device wait", ms);
  }

  // stdout through printf (WRITE fid 1).
  printf("hello from printf: %d %s %.3f\n", 42, "semihost", 3.14159);
  fflush(stdout);
  fprintf(stderr, "this line went to stderr\n");
  LOG_INFO("isatty(1) = %d", isatty(1));
  // WRITE with a reply (bit 31 clear), as the selftest's one check of that path.
  static const char replied[] = "this line was written with a reply\n";
  uint8_t wa[4 + sizeof(replied) - 1] = {1, 0, 0, 0};
  memcpy(wa + 4, replied, sizeof(replied) - 1);
  r = sh_rerr(SH_WRITE, wa, sizeof(wa), NULL);
  if (r == (int)sizeof(replied) - 1) {
    LOG_INFO("WRITE with reply: %d bytes", r);
  } else {
    LOG_ERROR("WRITE with reply: got %d, want %d", r, (int)sizeof(replied) - 1);
  }

  // Files (sandboxed to the host's files directory).
  FILE *fp = fopen("uclog-test.txt", "w");
  if (fp == NULL) {
    LOG_WARN("fopen(w) failed: errno %d (does the host have a files directory?)", errno);
  } else {
    fprintf(fp, "written by %s at %ld\n", "nucleo", (long)t); // newlib-nano: no %lld
    fclose(fp);
    fp = fopen("uclog-test.txt", "r");
    char line[64] = "";
    if (fp != NULL) {
      struct stat st;
      fstat(fileno(fp), &st);
      if (fgets(line, sizeof(line), fp) == NULL) line[0] = 0;
      fclose(fp);
      LOG_INFO("read back %ld bytes: \"%s\"", (long)st.st_size, line);
    } else {
      LOG_ERROR("fopen(r) failed: errno %d", errno);
    }
  }
  r = mkdir("uclog-dir", 0755);
  LOG_INFO("mkdir: %d errno %d", r, r < 0 ? errno : 0);

  // Errors.
  errno = 0;
  expect_err("open ../escape", _open("../escape.txt", O_RDONLY), EACCES);
  errno = 0;
  expect_err("close bad fid", _close(77), EBADF);
  errno = 0;
  expect_err("lseek", _lseek(1, 0, SEEK_SET), ENOSYS);
  errno = 0;
  expect_err("unknown op 0x55", sh_rerr(0x55, NULL, 0, NULL), ENOSYS);
  errno = 0;
  expect_err("malformed ISATTY", sh_rerr(SH_ISATTY, "ab", 2, NULL), EINVAL);
  errno = 0;
  expect_err("write to a closed fid", sh_rerr(SH_WRITE, (const uint32_t[]){55}, 4, NULL), EBADF);
  size_t len;
  errno = 0;
  r = sh_rerr(SH_FSTAT, (const int32_t[]){99}, 4, &len);
  expect_err("fstat bad fid", r, EBADF);
  if (len != 48) LOG_ERROR("fstat error reply is %zu bytes, want 48", len);

  LOG_INFO("semihost selftest done");
}
