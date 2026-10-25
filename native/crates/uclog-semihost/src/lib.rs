//! Sandboxed port-63 semihost handler (protocol.md §5.5).

pub mod clock;

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use uclog_proto::semihost::{self as sh, errno, oflag, Request, Stat};

pub use clock::{SystemClock, TimeSource};

/// Largest READ payload returned in one reply, so the reply fits in one frame (protocol.md §2.4:
/// 1443 bytes of port data, less the 8-byte prefix).
pub const MAX_READ: usize = 1024;
const _: () = assert!(MAX_READ + 8 <= uclog_proto::MAX_PORT_DATA);

/// What the handler does with non-time ops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Full handler (clients, hub `--syscalls`).
    Full,
    /// Hub default handler: time ops only; everything else → ENOSYS.
    TimeOnly,
}

/// Where stdio goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stdio {
    /// stdout/stderr are returned as [`Outcome::console`]; stdin reads wait for
    /// [`Handler::stdin`].
    Interactive,
    /// stdout/stderr discarded; stdin is EOF (hub `--syscalls`).
    Discard,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub mode: Mode,
    pub stdio: Stdio,
    /// Sandbox root. `None` → file ops fail with EACCES.
    pub files_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config { mode: Mode::Full, stdio: Stdio::Interactive, files_dir: None }
    }
}

/// Console output produced by a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleOut {
    /// 1 = stdout, 2 = stderr.
    pub fid: u32,
    pub data: Vec<u8>,
}

/// Result of handling one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Send `reply` (if any) on port 63; show `console` (if any).
    Done { reply: Option<Vec<u8>>, console: Option<ConsoleOut> },
    /// A stdin READ: call [`Handler::stdin`] when a line is available.
    WaitStdin { max: usize },
}

impl Outcome {
    fn reply(r: Vec<u8>) -> Self {
        Outcome::Done { reply: Some(r), console: None }
    }
}

struct Handle {
    file: File,
    path: PathBuf,
}

/// Per-session handler state.
pub struct Handler {
    cfg: Config,
    clock: Box<dyn TimeSource>,
    files: HashMap<i32, Handle>,
    next_fid: i32,
}

impl Handler {
    pub fn new(cfg: Config) -> Self {
        Self::with_clock(cfg, Box::new(SystemClock::new()))
    }

    pub fn with_clock(cfg: Config, clock: Box<dyn TimeSource>) -> Self {
        Handler { cfg, clock, files: HashMap::new(), next_fid: 3 }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Change the sandbox root (closes open files).
    pub fn set_files_dir(&mut self, dir: Option<PathBuf>) {
        self.files.clear();
        self.cfg.files_dir = dir;
    }

    /// Answer a time op directly (hubs always do this, D31).
    pub fn time_reply(&self, op: u8) -> Option<Vec<u8>> {
        sh::time_reply(op, &self.clock.sample())
    }

    /// Handle one request frame (port-63 payload).
    pub fn handle(&mut self, frame: &[u8]) -> Outcome {
        let op = frame.first().copied().unwrap_or(0xFF);
        let req = match sh::parse_request(frame) {
            Ok(r) => r,
            Err(_) => return Outcome::reply(sh::error_reply(op, errno::EINVAL)),
        };
        if let Some(r) = self.time_reply(op) {
            return Outcome::reply(r);
        }
        if self.cfg.mode == Mode::TimeOnly {
            return match req {
                Request::Write { reply: false, .. } => Outcome::Done { reply: None, console: None },
                _ => Outcome::reply(sh::error_reply(op, errno::ENOSYS)),
            };
        }
        let err = |e: i32| Outcome::reply(sh::error_reply(op, e));
        match req {
            Request::Isatty { fid } => match fid {
                0..=2 => Outcome::reply(sh::reply(1, 0)),
                f if self.files.contains_key(&f) => Outcome::reply(sh::reply(0, 0)),
                _ => err(errno::EBADF),
            },
            Request::Fstat { fid } => match fid {
                0..=2 => Outcome::reply(sh::fstat_reply(&Stat { mode: 0o020_666, nlink: 1, ..Default::default() })),
                f => match self.files.get(&f) {
                    Some(h) => match h.file.metadata() {
                        Ok(m) => Outcome::reply(sh::fstat_reply(&stat_of(&m))),
                        Err(e) => err(map_err(&e)),
                    },
                    None => err(errno::EBADF),
                },
            },
            Request::Close { fid } => match fid {
                0..=2 => Outcome::reply(sh::reply(0, 0)),
                f => match self.files.remove(&f) {
                    Some(_) => Outcome::reply(sh::reply(0, 0)),
                    None => err(errno::EBADF),
                },
            },
            Request::Open { flags, path } => match self.open(flags, &path) {
                Ok(fid) => Outcome::reply(sh::reply(fid, 0)),
                Err(e) => err(e),
            },
            Request::Mkdir { mode, path } => match self.resolve(&path) {
                Ok(p) => match mkdir(&p, mode) {
                    Ok(()) => Outcome::reply(sh::reply(0, 0)),
                    Err(e) => err(map_err(&e)),
                },
                Err(e) => err(e),
            },
            Request::Write { fid, reply, data } => {
                let (res, console) = match fid {
                    1 | 2 => {
                        let n = data.len() as i32;
                        let c = (self.cfg.stdio == Stdio::Interactive).then_some(ConsoleOut { fid, data });
                        (Ok(n), c)
                    }
                    f => match self.files.get_mut(&(f as i32)) {
                        Some(h) => (h.file.write_all(&data).map(|_| data.len() as i32).map_err(|e| map_err(&e)), None),
                        None => (Err(errno::EBADF), None),
                    },
                };
                let r = match res {
                    Ok(n) => sh::reply(n, 0),
                    Err(e) => sh::error_reply(op, e),
                };
                Outcome::Done { reply: reply.then_some(r), console }
            }
            Request::Read { fid, len } => {
                let max = (len as usize).min(MAX_READ);
                match fid {
                    0 => match self.cfg.stdio {
                        Stdio::Interactive => Outcome::WaitStdin { max },
                        Stdio::Discard => Outcome::reply(sh::read_reply(&[])),
                    },
                    f => match self.files.get_mut(&f) {
                        Some(h) => {
                            let mut buf = vec![0; max];
                            match h.file.read(&mut buf) {
                                Ok(n) => Outcome::reply(sh::read_reply(&buf[..n])),
                                Err(e) => err(map_err(&e)),
                            }
                        }
                        None => err(errno::EBADF),
                    },
                }
            }
            // Time ops are answered above.
            Request::TimeOfDay
            | Request::Clock
            | Request::Time
            | Request::Elapsed
            | Request::TickFreq
            | Request::Unsupported { .. }
            | Request::Unknown { .. } => err(errno::ENOSYS),
        }
    }

    /// Complete a pending stdin READ with a typed line. A trailing `\r` becomes `\n`.
    pub fn stdin(&self, line: &[u8], max: usize) -> Vec<u8> {
        let mut d = line.to_vec();
        if d.last() == Some(&b'\r') {
            *d.last_mut().unwrap() = b'\n';
        }
        d.truncate(max);
        sh::read_reply(&d)
    }

    /// Paths of currently open files (for display).
    pub fn open_files(&self) -> Vec<(i32, PathBuf)> {
        let mut v: Vec<_> = self.files.iter().map(|(k, h)| (*k, h.path.clone())).collect();
        v.sort();
        v
    }

    fn open(&mut self, flags: i32, path: &str) -> Result<i32, i32> {
        if self.files.len() >= 64 {
            return Err(errno::EMFILE);
        }
        let create = flags & oflag::CREAT != 0;
        let p = self.resolve(path)?;
        let mut o = OpenOptions::new();
        match flags & oflag::ACCMODE {
            oflag::RDONLY => o.read(true),
            oflag::WRONLY => o.write(true),
            _ => o.read(true).write(true),
        };
        if flags & oflag::APPEND != 0 {
            o.append(true);
        }
        if flags & oflag::TRUNC != 0 {
            o.truncate(true);
        }
        if create && flags & oflag::EXCL != 0 {
            o.create_new(true);
        } else if create {
            o.create(true);
        }
        // `p` has no symlinks left; refuse one that appeared since (protocol.md §5.5.5).
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.custom_flags(libc::O_NOFOLLOW);
        }
        // Windows: open a link that appeared since as itself, rather than following it.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            o.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        }
        let file = o.open(&p).map_err(|e| map_err(&e))?;
        let fid = self.next_fid;
        self.next_fid = if self.next_fid >= 0x7FFF_FFF0 { 3 } else { self.next_fid + 1 };
        self.files.insert(fid, Handle { file, path: p });
        Ok(fid)
    }

    /// Resolve `path` inside the sandbox to a path without symlinks. Absolute paths, `..`
    /// escapes, symlinks pointing outside (or nowhere) and names that are not portable fail with
    /// EACCES (protocol.md §5.5.5). `\` separates components on every platform.
    fn resolve(&self, path: &str) -> Result<PathBuf, i32> {
        // Canonicalized on each call, so a directory created after the session started works.
        let root = self.cfg.files_dir.as_ref().ok_or(errno::EACCES)?.canonicalize().map_err(|_| errno::EACCES)?;
        if path.is_empty() {
            return Err(errno::ENOENT);
        }
        let path = path.replace('\\', "/");
        let mut lexical = PathBuf::new();
        for c in Path::new(&path).components() {
            match c {
                Component::Normal(s) => {
                    if !s.to_str().is_some_and(portable_name) {
                        return Err(errno::EACCES);
                    }
                    lexical.push(s)
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if !lexical.pop() {
                        return Err(errno::EACCES);
                    }
                }
                Component::RootDir | Component::Prefix(_) => return Err(errno::EACCES),
            }
        }
        let Some(name) = lexical.file_name().map(PathBuf::from) else { return Ok(root) };
        // The directories must exist; canonicalizing them catches symlinks that leave the sandbox.
        let parent = root.join(lexical.parent().unwrap_or(Path::new(""))).canonicalize().map_err(|e| map_err(&e))?;
        let mut full = parent.join(name);
        // The last component may not exist yet (O_CREAT, MKDIR). If it is a symlink, it must
        // resolve inside the sandbox: a dangling one is refused, or O_CREAT would follow it.
        if full.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
            full = full.canonicalize().map_err(|_| errno::EACCES)?;
        }
        if !full.starts_with(&root) {
            return Err(errno::EACCES);
        }
        Ok(full)
    }
}

/// A name every host can store as a plain file (protocol.md §5.5.5). Refused, so a device sees the
/// same result on every host: `:` (Windows drive letters and alternate data streams), the Windows
/// device names with any extension (`CON`, `NUL`, `COM1`, `aux.txt`), a trailing dot or space
/// (Windows drops them), and control characters and `<>"|?*`.
pub fn portable_name(s: &str) -> bool {
    const DEVICES: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    if s.is_empty() || s.ends_with(['.', ' ']) || s.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c)) {
        return false;
    }
    // Windows matches the part before the first dot, ignoring trailing spaces.
    let stem = s.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
    let numbered = |p: &str| {
        stem.strip_prefix(p).is_some_and(|n| matches!(n, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"))
    };
    !(DEVICES.contains(&stem.as_str()) || numbered("COM") || numbered("LPT"))
}

fn stat_of(m: &std::fs::Metadata) -> Stat {
    let secs = |t: io::Result<std::time::SystemTime>| {
        t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as u32).unwrap_or(0)
    };
    #[cfg(unix)]
    let (perm, ino, nlink, uid, gid) = {
        use std::os::unix::fs::MetadataExt;
        (m.mode() & 0o7777, m.ino() as u32, m.nlink() as u32, m.uid(), m.gid())
    };
    #[cfg(not(unix))]
    let (perm, ino, nlink, uid, gid) = (if m.permissions().readonly() { 0o444 } else { 0o666 }, 0, 1, 0, 0);
    let kind = if m.is_dir() { 0o040_000 } else { 0o100_000 };
    Stat {
        mode: kind | perm,
        ino,
        nlink,
        uid,
        gid,
        size: m.len().min(u32::MAX as u64) as u32,
        atime: secs(m.accessed()),
        mtime: secs(m.modified()),
        ctime: secs(m.created()),
    }
}

fn mkdir(p: &Path, mode: i32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(mode as u32 & 0o7777).create(p)
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        std::fs::create_dir(p)
    }
}

/// Map a host I/O error to a newlib errno (D25).
pub fn map_err(e: &io::Error) -> i32 {
    use io::ErrorKind as K;
    match e.kind() {
        K::NotFound => errno::ENOENT,
        K::PermissionDenied => errno::EACCES,
        K::AlreadyExists => errno::EEXIST,
        K::InvalidInput | K::InvalidData => errno::EINVAL,
        K::IsADirectory => errno::EISDIR,
        K::NotADirectory => errno::ENOTDIR,
        K::DirectoryNotEmpty => errno::ENOTEMPTY,
        K::StorageFull => errno::ENOSPC,
        K::ReadOnlyFilesystem => errno::EROFS,
        K::Interrupted => errno::EINTR,
        K::WouldBlock => errno::EAGAIN,
        K::OutOfMemory => errno::ENOMEM,
        K::InvalidFilename => errno::ENAMETOOLONG,
        K::NotSeekable => errno::ESPIPE,
        _ => errno::EIO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sh::op;

    struct Fixed;
    impl TimeSource for Fixed {
        fn sample(&self) -> sh::TimeSample {
            sh::TimeSample { ticks: 1_000_000_000, freq: 1_000_000_000, wall: 100.5 }
        }
    }

    fn req(o: u8, args: &[u8]) -> Vec<u8> {
        let mut v = vec![o];
        v.extend_from_slice(args);
        v
    }

    fn open_req(flags: i32, path: &str) -> Vec<u8> {
        let mut a = flags.to_le_bytes().to_vec();
        a.extend_from_slice(path.as_bytes());
        req(op::OPEN, &a)
    }

    fn reply(o: Outcome) -> Vec<u8> {
        match o {
            Outcome::Done { reply: Some(r), .. } => r,
            o => panic!("{o:?}"),
        }
    }

    fn r_errno(r: &[u8]) -> (i32, i32) {
        (i32::from_le_bytes(r[..4].try_into().unwrap()), i32::from_le_bytes(r[4..8].try_into().unwrap()))
    }

    fn handler(dir: &Path) -> Handler {
        Handler::with_clock(Config { files_dir: Some(dir.into()), ..Default::default() }, Box::new(Fixed))
    }

    /// `link` → `target`; false where the OS refuses (Windows without the symlink privilege or
    /// Developer Mode).
    fn symlink(target: &Path, link: &Path, dir: bool) -> bool {
        #[cfg(unix)]
        {
            let _ = dir;
            std::os::unix::fs::symlink(target, link).unwrap();
            true
        }
        #[cfg(windows)]
        {
            let r = if dir { std::os::windows::fs::symlink_dir(target, link) } else { std::os::windows::fs::symlink_file(target, link) };
            r.is_ok()
        }
    }

    #[test]
    fn file_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let mut h = handler(d.path());
        let (fid, e) = r_errno(&reply(h.handle(&open_req(oflag::WRONLY | oflag::CREAT | oflag::TRUNC, "out.txt"))));
        assert_eq!((e, fid >= 3), (0, true));
        let mut w = (fid as u32).to_le_bytes().to_vec();
        w.extend_from_slice(b"hello");
        assert_eq!(r_errno(&reply(h.handle(&req(op::WRITE, &w)))), (5, 0));
        assert_eq!(reply(h.handle(&req(op::FSTAT, &fid.to_le_bytes()))).len(), 48);
        assert_eq!(r_errno(&reply(h.handle(&req(op::CLOSE, &fid.to_le_bytes())))), (0, 0));
        assert_eq!(std::fs::read(d.path().join("out.txt")).unwrap(), b"hello");

        let (fid, _) = r_errno(&reply(h.handle(&open_req(oflag::RDONLY, "out.txt"))));
        let mut a = fid.to_le_bytes().to_vec();
        a.extend_from_slice(&100u32.to_le_bytes());
        let r = reply(h.handle(&req(op::READ, &a)));
        assert_eq!((r_errno(&r), &r[8..]), ((5, 0), &b"hello"[..]));
        assert_eq!(r_errno(&reply(h.handle(&req(op::CLOSE, &fid.to_le_bytes())))), (0, 0));
        assert_eq!(r_errno(&reply(h.handle(&req(op::CLOSE, &fid.to_le_bytes())))), (-1, errno::EBADF));
    }

    #[test]
    fn sandbox() {
        let d = tempfile::tempdir().unwrap();
        let mut h = handler(d.path());
        for p in ["../x", "/etc/passwd", "a/../../x", "a\\..\\..\\x", "\\x", "C:/x", "C:x", "\\\\?\\C:\\x", "//srv/share/x"] {
            assert_eq!(r_errno(&reply(h.handle(&open_req(oflag::RDONLY, p)))), (-1, errno::EACCES), "{p}");
        }
        assert_eq!(r_errno(&reply(h.handle(&open_req(oflag::RDONLY, "missing")))), (-1, errno::ENOENT));
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"s").unwrap();
        if symlink(outside.path(), &d.path().join("link"), true) {
            assert_eq!(r_errno(&reply(h.handle(&open_req(oflag::RDONLY, "link/secret")))), (-1, errno::EACCES));
        }
        let mut no_dir = Handler::with_clock(Config::default(), Box::new(Fixed));
        assert_eq!(r_errno(&reply(no_dir.handle(&open_req(oflag::RDONLY, "x")))), (-1, errno::EACCES));
        let mut m = 0o755i32.to_le_bytes().to_vec();
        m.extend_from_slice(b"sub");
        assert_eq!(r_errno(&reply(h.handle(&req(op::MKDIR, &m)))), (0, 0));
        assert!(d.path().join("sub").is_dir());
    }

    /// A dangling symlink must not be followed by O_CREAT (it would create a file outside the
    /// sandbox); a symlink that stays inside works.
    #[test]
    fn sandbox_symlinks() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("pwned");
        let secret = outside.path().join("secret");
        std::fs::write(&secret, b"s").unwrap();
        if !symlink(&target, &d.path().join("dangling"), false) {
            eprintln!("skipped: this account cannot create symlinks");
            return;
        }
        assert!(symlink(&secret, &d.path().join("hosts"), false));
        std::fs::write(d.path().join("real.txt"), b"in").unwrap();
        assert!(symlink(Path::new("real.txt"), &d.path().join("inside"), false));
        let mut h = handler(d.path());
        let creat = oflag::WRONLY | oflag::CREAT | oflag::TRUNC;
        assert_eq!(r_errno(&reply(h.handle(&open_req(creat, "dangling")))), (-1, errno::EACCES));
        assert_eq!(r_errno(&reply(h.handle(&open_req(creat, "sub/../dangling")))), (-1, errno::EACCES));
        assert!(!target.exists());
        assert_eq!(r_errno(&reply(h.handle(&open_req(oflag::RDONLY, "hosts")))), (-1, errno::EACCES));
        let (fid, e) = r_errno(&reply(h.handle(&open_req(oflag::RDONLY, "inside"))));
        assert_eq!((e, fid >= 3), (0, true));
    }

    /// A directory junction (Windows; no privilege needed to create one) that leads out of the
    /// sandbox is refused, as a directory symlink is.
    #[cfg(windows)]
    #[test]
    fn sandbox_junctions() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"s").unwrap();
        let ok = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(d.path().join("j"))
            .arg(outside.path())
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "mklink /J");
        let mut h = handler(d.path());
        let creat = oflag::WRONLY | oflag::CREAT | oflag::TRUNC;
        assert_eq!(r_errno(&reply(h.handle(&open_req(oflag::RDONLY, "j/secret")))), (-1, errno::EACCES));
        assert_eq!(r_errno(&reply(h.handle(&open_req(creat, "j/new")))), (-1, errno::EACCES));
        assert!(!outside.path().join("new").exists());
        let mut m = 0o755i32.to_le_bytes().to_vec();
        m.extend_from_slice(b"j/sub");
        assert_eq!(r_errno(&reply(h.handle(&req(op::MKDIR, &m)))), (-1, errno::EACCES));
    }

    /// Names a Windows host could not store as plain files are refused on every host.
    #[test]
    fn portable_names() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("plain.txt"), b"p").unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        let mut h = handler(d.path());
        let creat = oflag::WRONLY | oflag::CREAT | oflag::TRUNC;
        for p in [
            "CON", "nul", "Nul.txt", "aux.tar.gz", "PRN ", "COM1", "lpt9.log", "COM¹", "CONIN$", "conout$.x",
            "plain.txt:stream", "sub/C:x", "trail.", "trail ", "...", "q?.txt", "a*", "<x>", "\"q\"", "p|q",
            "x\u{1}y", "sub/NUL/x",
        ] {
            assert_eq!(r_errno(&reply(h.handle(&open_req(creat, p)))), (-1, errno::EACCES), "{p:?}");
        }
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 2, "nothing was created");
        for p in ["log-1.txt", "a.b.c", "COM10.txt", "CONSOLE", "nul_x", "con-fig", ".hidden", "sub\\x.txt", "LPT"] {
            let (fid, e) = r_errno(&reply(h.handle(&open_req(creat, p))));
            assert_eq!((e, fid >= 3), (0, true), "{p:?}");
        }
        assert!(d.path().join("sub").join("x.txt").exists());
    }

    /// The sandbox root may be created after the handler.
    #[test]
    fn root_created_later() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("later");
        let mut h = handler(&root);
        let creat = oflag::WRONLY | oflag::CREAT;
        assert_eq!(r_errno(&reply(h.handle(&open_req(creat, "x")))), (-1, errno::EACCES));
        std::fs::create_dir(&root).unwrap();
        let (fid, e) = r_errno(&reply(h.handle(&open_req(creat, "x"))));
        assert_eq!((e, fid >= 3), (0, true));
    }

    /// EMFILE is reported before the file is opened, so O_CREAT|O_TRUNC has no effect.
    #[test]
    fn emfile_before_open() {
        let d = tempfile::tempdir().unwrap();
        let mut h = handler(d.path());
        for _ in 0..64 {
            assert_eq!(r_errno(&reply(h.handle(&open_req(oflag::WRONLY | oflag::CREAT, "a")))).1, 0);
        }
        std::fs::write(d.path().join("keep"), b"data").unwrap();
        let r = r_errno(&reply(h.handle(&open_req(oflag::WRONLY | oflag::CREAT | oflag::TRUNC, "keep"))));
        assert_eq!(r, (-1, errno::EMFILE));
        assert_eq!(std::fs::read(d.path().join("keep")).unwrap(), b"data");
        assert_eq!(r_errno(&reply(h.handle(&open_req(oflag::WRONLY | oflag::CREAT, "new")))), (-1, errno::EMFILE));
        assert!(!d.path().join("new").exists());
    }

    #[test]
    fn stdio_time_and_errors() {
        let mut h = Handler::with_clock(Config::default(), Box::new(Fixed));
        let mut w = 1u32.to_le_bytes().to_vec();
        w.extend_from_slice(b"hi");
        assert_eq!(
            h.handle(&req(op::WRITE, &w)),
            Outcome::Done { reply: Some(sh::reply(2, 0)), console: Some(ConsoleOut { fid: 1, data: b"hi".to_vec() }) }
        );
        let mut w = (1u32 | sh::WRITE_NO_REPLY).to_le_bytes().to_vec();
        w.push(b'x');
        assert!(matches!(h.handle(&req(op::WRITE, &w)), Outcome::Done { reply: None, console: Some(_) }));
        let mut a = 0i32.to_le_bytes().to_vec();
        a.extend_from_slice(&10u32.to_le_bytes());
        assert_eq!(h.handle(&req(op::READ, &a)), Outcome::WaitStdin { max: 10 });
        assert_eq!(h.stdin(b"ok\r", 10), sh::read_reply(b"ok\n"));
        assert_eq!(reply(h.handle(&[op::SYS_CLOCK])), 100u32.to_le_bytes());
        assert_eq!(
            reply(h.handle(&[op::TIMEOFDAY])),
            [100u64.to_le_bytes().to_vec(), 500_000u32.to_le_bytes().to_vec()].concat()
        );
        assert_eq!(reply(h.handle(&[0x20])), sh::error_reply(0x20, errno::ENOSYS));
        assert_eq!(reply(h.handle(&[op::CLOSE, 1])), sh::error_reply(op::CLOSE, errno::EINVAL));
        assert_eq!(reply(h.handle(&req(op::FSTAT, &99i32.to_le_bytes()))).len(), 48);

        let mut t = Handler::with_clock(Config { mode: Mode::TimeOnly, ..Default::default() }, Box::new(Fixed));
        assert_eq!(reply(t.handle(&[op::SYS_TICKFREQ])), 1_000_000_000u64.to_le_bytes());
        assert_eq!(reply(t.handle(&open_req(0, "x"))), sh::error_reply(op::OPEN, errno::ENOSYS));
    }
}
