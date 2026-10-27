// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Semihost on Zephyr (CONFIG_UC_SYSCALLS, issue #2): the host's files directory as a Zephyr file
// system mounted at CONFIG_UC_SYSCALLS_MOUNT (fs_open() and the rest; open()/fopen() too with the
// POSIX file API), and stdout/stdin through the host's semihost console. The requests are
// semihost.c's. Zephyr's newlib defines the newlib syscalls itself, so syscalls.c is not used.
//
// The host has no LSEEK, STAT, UNLINK or RENAME (spec/protocol.md §5.5.2). Positions are kept
// here: seeks and tell() work, but reads and writes only where the host's file is (else
// -ENOTSUP). stat() opens the file and FSTATs it. Unlink, rename and truncation (except to 0 at
// open) return -ENOTSUP.

#include <errno.h>
#include <string.h>

#include <zephyr/fs/fs.h>
#include <zephyr/fs/fs_sys.h>
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/libc-hooks.h>

#include "semihost.h"

#define FS_TYPE_UCLOG (FS_TYPE_EXTERNAL_BASE + 0x75)

// Declared by <zephyr/sys/libc-hooks.h> only with some C libraries and Zephyr versions.
void __stdin_hook_install(unsigned char (*hook)(void));
void __stdout_hook_install(int (*hook)(int));

// "/host/a/b" → "a/b": the host resolves paths in its files directory.
static const char *host_path(const char *fs_path) {
  const char *p = fs_path + strlen(CONFIG_UC_SYSCALLS_MOUNT);
  while (*p == '/') p++;
  return p;
}

// Open files: the host's fid and position. The host has no LSEEK, so a seek only moves `seek`
// here; reads and writes work while it equals `pos`. That covers tell() and Zephyr's fstat()
// (seek to the end, tell, seek back).
typedef struct {
  bool used;
  int fid;
  off_t pos;  // the host's position
  off_t seek; // the position asked for
} sh_file_t;

static sh_file_t files[CONFIG_UC_SYSCALLS_MAX_FILES];
static K_MUTEX_DEFINE(files_mutex);

static sh_file_t *file_of(const struct fs_file_t *f) {
  return (sh_file_t *)f->filep;
}

static int host_size(int fid, off_t *size) {
  uclog_sh_stat_t st;
  if (uclog_sh_fstat(fid, &st) < 0) return -errno;
  *size = (off_t)st.size;
  return 0;
}

static int sh_open(struct fs_file_t *f, const char *fs_path, fs_mode_t flags) {
  int w;
  switch (flags & FS_O_MODE_MASK) {
  case FS_O_WRITE:
    w = UCLOG_SH_O_WRONLY;
    break;
  case FS_O_RDWR:
    w = UCLOG_SH_O_RDWR;
    break;
  default:
    w = UCLOG_SH_O_RDONLY;
    break;
  }
  if (flags & FS_O_CREATE) w |= UCLOG_SH_O_CREAT;
  if (flags & FS_O_APPEND) w |= UCLOG_SH_O_APPEND;
  if (flags & FS_O_TRUNC) w |= UCLOG_SH_O_TRUNC;

  k_mutex_lock(&files_mutex, K_FOREVER);
  sh_file_t *sf = NULL;
  for (size_t i = 0; i < ARRAY_SIZE(files) && sf == NULL; i++) {
    if (!files[i].used) sf = &files[i];
  }
  if (sf != NULL) sf->used = true;
  k_mutex_unlock(&files_mutex);
  if (sf == NULL) return -ENFILE;

  int fid = uclog_sh_open(host_path(fs_path), w);
  if (fid < 0) {
    int e = errno;
    sf->used = false;
    return -e;
  }
  sf->fid = fid;
  sf->pos = 0;
  if (flags & FS_O_APPEND) (void)host_size(fid, &sf->pos); // writes go to the end
  sf->seek = sf->pos;
  f->filep = sf;
  return 0;
}

static ssize_t sh_read(struct fs_file_t *f, void *dest, size_t n) {
  sh_file_t *sf = file_of(f);
  if (sf->seek != sf->pos) return -ENOTSUP; // the host cannot seek
  int r = uclog_sh_read(sf->fid, dest, n);
  if (r < 0) return -errno;
  sf->pos += r;
  sf->seek = sf->pos;
  return r;
}

static ssize_t sh_write(struct fs_file_t *f, const void *src, size_t n) {
  sh_file_t *sf = file_of(f);
  if (sf->seek != sf->pos) return -ENOTSUP; // the host cannot seek
  size_t done = 0;
  while (done < n) {
    int r = uclog_sh_write(sf->fid, (const uint8_t *)src + done, n - done);
    if (r <= 0) break;
    done += (size_t)r;
  }
  if (done == 0 && n > 0) return -EIO;
  sf->pos += (off_t)done;
  sf->seek = sf->pos;
  return (ssize_t)done;
}

static int sh_lseek(struct fs_file_t *f, off_t off, int whence) {
  sh_file_t *sf = file_of(f);
  off_t to = 0;
  switch (whence) {
  case FS_SEEK_SET:
    to = off;
    break;
  case FS_SEEK_CUR:
    to = sf->seek + off;
    break;
  case FS_SEEK_END: {
    int r = host_size(sf->fid, &to);
    if (r < 0) return r;
    to += off;
    break;
  }
  default:
    return -EINVAL;
  }
  if (to < 0) return -EINVAL;
  sf->seek = to;
  return 0;
}

static off_t sh_tell(struct fs_file_t *f) {
  return file_of(f)->seek;
}

// fs_open() with FS_O_TRUNC calls this with 0 after the open, which the host already truncated.
static int sh_truncate(struct fs_file_t *f, off_t length) {
  ARG_UNUSED(f);
  return length == 0 ? 0 : -ENOTSUP;
}

static int sh_sync(struct fs_file_t *f) {
  ARG_UNUSED(f);
  return 0;
}

static int sh_close(struct fs_file_t *f) {
  sh_file_t *sf = file_of(f);
  int r = uclog_sh_close(sf->fid) < 0 ? -errno : 0;
  sf->used = false;
  return r;
}

static int sh_mkdir(struct fs_mount_t *mp, const char *name) {
  ARG_UNUSED(mp);
  return uclog_sh_mkdir(host_path(name), 0755) < 0 ? -errno : 0;
}

// The host has no STAT: open the path and FSTAT it (a directory cannot be opened: EISDIR).
static int sh_stat(struct fs_mount_t *mp, const char *path, struct fs_dirent *entry) {
  ARG_UNUSED(mp);
  const char *base = strrchr(path, '/');
  strncpy(entry->name, base != NULL ? base + 1 : path, sizeof(entry->name) - 1);
  entry->name[sizeof(entry->name) - 1] = '\0';
  entry->size = 0;
  int fid = uclog_sh_open(host_path(path), UCLOG_SH_O_RDONLY);
  if (fid < 0) {
    if (errno != EISDIR) return -errno;
    entry->type = FS_DIR_ENTRY_DIR;
    return 0;
  }
  uclog_sh_stat_t st;
  int r = uclog_sh_fstat(fid, &st) < 0 ? -errno : 0;
  (void)uclog_sh_close(fid);
  if (r < 0) return r;
  entry->type = (st.mode & 0170000) == 0040000 ? FS_DIR_ENTRY_DIR : FS_DIR_ENTRY_FILE;
  entry->size = st.size;
  return 0;
}

static int sh_mount(struct fs_mount_t *mp) {
  ARG_UNUSED(mp);
  return 0;
}

static const struct fs_file_system_t sh_fs = {
  .open = sh_open,
  .read = sh_read,
  .write = sh_write,
  .lseek = sh_lseek,
  .tell = sh_tell,
  .truncate = sh_truncate,
  .sync = sh_sync,
  .close = sh_close,
  .mount = sh_mount,
  .unmount = sh_mount,
  .mkdir = sh_mkdir,
  .stat = sh_stat,
};

static struct fs_mount_t sh_mnt = {
  .type = FS_TYPE_UCLOG,
  .mnt_point = CONFIG_UC_SYSCALLS_MOUNT,
};

// ---------------------------------------------------------------------------------------------
// stdout (and stderr, the same stream on picolibc) and stdin

static K_MUTEX_DEFINE(out_mutex);
static char out[128];
static size_t out_n;

// Lines go to the host whole (one WRITE each); ISRs cannot wait for buffer room, so their output
// is dropped.
static int stdout_hook(int c) {
  if (k_is_in_isr()) return c;
  k_mutex_lock(&out_mutex, K_FOREVER);
  out[out_n++] = (char)c;
  if (c == '\n' || out_n == sizeof(out)) {
    uclog_sh_write(1, out, out_n);
    out_n = 0;
  }
  k_mutex_unlock(&out_mutex);
  return c;
}

static uint8_t in[64];
static size_t in_n, in_i;

// One character; a READ fetches the rest of the line. With no host (or EOF) it waits.
static unsigned char stdin_hook(void) {
  while (in_i == in_n) {
    int r = uclog_sh_read(0, in, sizeof(in));
    if (r > 0) {
      in_n = (size_t)r;
      in_i = 0;
    } else {
      k_msleep(100);
    }
  }
  return in[in_i++];
}

static int uclog_semihost_init(void) {
  __stdout_hook_install(stdout_hook);
  __stdin_hook_install(stdin_hook);
  int r = fs_register(FS_TYPE_UCLOG, &sh_fs);
  if (r == 0) r = fs_mount(&sh_mnt);
  if (r < 0) LOG_ERROR("semihost: mount %s failed: %d", CONFIG_UC_SYSCALLS_MOUNT, r);
  return 0;
}

// After the log port (APPLICATION 0).
SYS_INIT(uclog_semihost_init, APPLICATION, 1);
