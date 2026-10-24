#include "semihost_test.h"

#include <errno.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <unistd.h>

#include "crc32c.h"
#include "log.h"

#define SH_PORT 63

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

static uint8_t req[SH_BUF_SIZE];
static uint8_t rsp[SH_BUF_SIZE];

static bool host(void) {
  bool ready;
  log_is_host_ready(&ready);
  return ready;
}

size_t sh_call(uint8_t op, const void *args, size_t n, void *reply, size_t cap) {
  if (!host() || n + 1 > sizeof(req)) return 0;
  req[0] = op;
  memcpy(req + 1, args, n);
  log_tx(SH_PORT, req, n + 1);
  size_t r = log_rx(SH_PORT, reply, cap);
  if (r == 0) LOG_ERROR("semihost op {enum:sh_op}%d: no reply", op);
  return r < cap ? r : cap;
}

// `<i32 r, i32 errno>` replies: returns r and the errno.
static int32_t sh_rerr(uint8_t op, const void *args, size_t n, int32_t *err, size_t *len) {
  size_t got = sh_call(op, args, n, rsp, sizeof(rsp));
  if (len) *len = got;
  if (got < 8) {
    *err = EIO;
    return -1;
  }
  int32_t r;
  memcpy(&r, rsp, 4);
  memcpy(err, rsp + 4, 4);
  return r;
}

static uint64_t sh_u64(uint8_t op) {
  uint64_t v = 0;
  if (sh_call(op, NULL, 0, rsp, sizeof(rsp)) >= 8) memcpy(&v, rsp, 8);
  return v;
}

static uint32_t sh_clock(void) {
  uint32_t v = 0;
  if (sh_call(SH_CLOCK, NULL, 0, rsp, sizeof(rsp)) >= 4) memcpy(&v, rsp, 4);
  return v;
}

static void expect_err(const char *what, int32_t r, int32_t err, int32_t want) {
  if (r == -1 && err == want) {
    LOG_INFO("%s: -1 errno %d as expected", what, (int)err);
  } else {
    LOG_ERROR("%s: got %d errno %d, want -1 errno %d", what, (int)r, (int)err, (int)want);
  }
}

// Write `n` pattern bytes with write() (syscalls.c sends at most SH_BUF_SIZE - 5 bytes per WRITE:
// 1438 with the full frame size, the largest that fits; it returns a short count) and read them
// back with read() (READ requests of SH_BUF_SIZE - 8 bytes, answered with at most 1024). No stdio:
// newlib-nano's unbuffered fread() reads one byte per _read().
static void big_file(size_t n) {
  static uint8_t out[SH_BIG_FILE], in[SH_BIG_FILE];
  for (size_t i = 0; i < n; i++) out[i] = (uint8_t)(i * 7 + 1);
  uint32_t crc = crc32c_update(CRC32C_INIT, out, n);
  int fd = open("uclog-big.bin", O_WRONLY | O_CREAT | O_TRUNC, 0644);
  if (fd < 0) {
    LOG_WARN("open(w) failed: errno %d (does the host have a files directory?)", errno);
    return;
  }
  ssize_t w = 0, k;
  while ((size_t)w < n && (k = write(fd, out + w, n - (size_t)w)) > 0) w += k; // short writes
  close(fd);
  fd = open("uclog-big.bin", O_RDONLY);
  if (fd < 0) {
    LOG_ERROR("open(r) failed: errno %d", errno);
    return;
  }
  size_t r = 0, reads = 0;
  while (r < n) {
    ssize_t k = read(fd, in + r, n - r);
    if (k <= 0) break;
    r += (size_t)k;
    reads++;
  }
  close(fd);
  uint32_t crc2 = crc32c_update(CRC32C_INIT, in, r);
  if (w == (ssize_t)n && r == n && crc == crc2) {
    LOG_INFO("file round trip: %u bytes in %u reads, crc32c %08lx", (unsigned)n, (unsigned)reads,
             (unsigned long)crc);
  } else {
    LOG_ERROR("file round trip: wrote %d read %u of %u, crc32c %08lx vs %08lx", (int)w, (unsigned)r,
              (unsigned)n, (unsigned long)crc, (unsigned long)crc2);
  }
}

void sh_selftest(void) {
  if (!host()) {
    LOG_WARN("semihost selftest: no host");
    return;
  }
  LOG_INFO("semihost selftest start");

  // Time ops (TIMEOFDAY through newlib, the others raw).
  struct timeval tv;
  int r = gettimeofday(&tv, NULL);
  LOG_INFO("TIMEOFDAY r=%d sec=%lld usec=%ld", r, (long long)tv.tv_sec, (long)tv.tv_usec);
  int64_t t = (int64_t)sh_u64(SH_TIME);
  LOG_INFO("SYS_TIME %lld (delta to TIMEOFDAY %lld)", t, t - (long long)tv.tv_sec);
  uint64_t f = sh_u64(SH_TICKFREQ), e0 = sh_u64(SH_ELAPSED);
  uint32_t c0 = sh_clock();
  uint32_t t0 = log_port_ms();
  while (log_port_ms() - t0 < 200) log_poll();
  uint64_t e1 = sh_u64(SH_ELAPSED);
  uint32_t c1 = sh_clock();
  LOG_INFO("SYS_TICKFREQ %llu, SYS_ELAPSED delta %llu ticks over 200 ms", f, e1 - e0);
  LOG_INFO("SYS_CLOCK %lu -> %lu (delta %lu cs)", (unsigned long)c0, (unsigned long)c1,
           (unsigned long)(c1 - c0));
  if (f != 0) {
    double ms = (double)(e1 - e0) * 1000.0 / (double)f;
    LOG_INFO("host measured %.1f ms for a 200 ms device wait", ms);
  }

  // stdout/stderr through printf (WRITE with bit 31: no reply).
  printf("hello from printf: %d %s %.3f\n", 42, "semihost", 3.14159);
  fflush(stdout);
  fprintf(stderr, "this line went to stderr\n");
  LOG_INFO("isatty(1) = %d", isatty(1));

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
      LOG_INFO("read back %ld bytes (mode %lo): \"%s\"", (long)st.st_size, (unsigned long)st.st_mode, line);
    } else {
      LOG_ERROR("fopen(r) failed: errno %d", errno);
    }
  }
  big_file(SH_BUF_SIZE - 5); // one maximum WRITE
  big_file(SH_BIG_FILE);     // several WRITEs and READs
  r = mkdir("uclog-dir", 0755);
  LOG_INFO("mkdir: %d errno %d", r, r < 0 ? errno : 0);

  // WRITE with a reply (bit 31 clear): syscalls.c never asks for one.
  static const char replied[] = "this line was written with a reply\n";
  uint8_t wa[4 + sizeof(replied) - 1] = {1, 0, 0, 0};
  memcpy(wa + 4, replied, sizeof(replied) - 1);
  int32_t err;
  int32_t n = sh_rerr(SH_WRITE, wa, sizeof(wa), &err, NULL);
  if (n == (int32_t)sizeof(replied) - 1) {
    LOG_INFO("WRITE with reply: %d bytes", (int)n);
  } else {
    LOG_ERROR("WRITE with reply: got %d, want %d", (int)n, (int)sizeof(replied) - 1);
  }

  // Error replies.
  static const char escape[] = "\0\0\0\0../escape.txt";
  expect_err("open ../escape", sh_rerr(SH_OPEN, escape, sizeof(escape) - 1, &err, NULL), err, EACCES);
  expect_err("close bad fid", sh_rerr(SH_CLOSE, (const int32_t[]){77}, 4, &err, NULL), err, EBADF);
  expect_err("lseek", sh_rerr(SH_LSEEK, (const int32_t[]){1, 0, 0}, 12, &err, NULL), err, ENOSYS);
  expect_err("unknown op 0x55", sh_rerr(0x55, NULL, 0, &err, NULL), err, ENOSYS);
  expect_err("malformed ISATTY", sh_rerr(SH_ISATTY, "ab", 2, &err, NULL), err, EINVAL);
  expect_err("write to a closed fid", sh_rerr(SH_WRITE, (const uint32_t[]){55}, 4, &err, NULL), err, EBADF);
  size_t len;
  r = sh_rerr(SH_FSTAT, (const int32_t[]){99}, 4, &err, &len);
  expect_err("fstat bad fid", r, err, EBADF);
  if (len != 48) LOG_ERROR("fstat error reply is %zu bytes, want 48", len);

  LOG_INFO("semihost selftest done");
}
