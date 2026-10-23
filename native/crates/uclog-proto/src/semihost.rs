//! Port-63 semihost codec (protocol.md §5.5): the ucLog op set (legacy `semihost.py`) plus the
//! time ops. Not ARM semihosting.

use thiserror::Error;

pub mod op {
    pub const SYSTEM: u8 = 0;
    pub const RENAME: u8 = 1;
    pub const ISATTY: u8 = 2;
    pub const UNLINK: u8 = 3;
    pub const LINK: u8 = 4;
    pub const STAT: u8 = 5;
    pub const FSTAT: u8 = 6;
    pub const CLOSE: u8 = 7;
    pub const OPEN: u8 = 8;
    pub const WRITE: u8 = 9;
    pub const LSEEK: u8 = 10;
    pub const READ: u8 = 11;
    pub const MKDIR: u8 = 12;
    pub const TIMEOFDAY: u8 = 13;
    pub const SYS_CLOCK: u8 = 0x10;
    pub const SYS_TIME: u8 = 0x11;
    pub const SYS_ELAPSED: u8 = 0x30;
    pub const SYS_TICKFREQ: u8 = 0x31;
}

pub fn op_name(o: u8) -> Option<&'static str> {
    Some(match o {
        op::SYSTEM => "SYSTEM",
        op::RENAME => "RENAME",
        op::ISATTY => "ISATTY",
        op::UNLINK => "UNLINK",
        op::LINK => "LINK",
        op::STAT => "STAT",
        op::FSTAT => "FSTAT",
        op::CLOSE => "CLOSE",
        op::OPEN => "OPEN",
        op::WRITE => "WRITE",
        op::LSEEK => "LSEEK",
        op::READ => "READ",
        op::MKDIR => "MKDIR",
        op::TIMEOFDAY => "TIMEOFDAY",
        op::SYS_CLOCK => "SYS_CLOCK",
        op::SYS_TIME => "SYS_TIME",
        op::SYS_ELAPSED => "SYS_ELAPSED",
        op::SYS_TICKFREQ => "SYS_TICKFREQ",
        _ => return None,
    })
}

/// Time ops: answered by the hub whenever there is one (D31).
pub fn is_time_op(o: u8) -> bool {
    matches!(o, op::TIMEOFDAY | op::SYS_CLOCK | op::SYS_TIME | op::SYS_ELAPSED | op::SYS_TICKFREQ)
}

pub const WRITE_NO_REPLY: u32 = 0x8000_0000;

/// newlib `<sys/errno.h>` values (D25).
pub mod errno {
    pub const EPERM: i32 = 1;
    pub const ENOENT: i32 = 2;
    pub const EINTR: i32 = 4;
    pub const EIO: i32 = 5;
    pub const EBADF: i32 = 9;
    pub const EAGAIN: i32 = 11;
    pub const ENOMEM: i32 = 12;
    pub const EACCES: i32 = 13;
    pub const EEXIST: i32 = 17;
    pub const ENOTDIR: i32 = 20;
    pub const EISDIR: i32 = 21;
    pub const EINVAL: i32 = 22;
    pub const EMFILE: i32 = 24;
    pub const ENOSPC: i32 = 28;
    pub const ESPIPE: i32 = 29;
    pub const EROFS: i32 = 30;
    pub const ENOSYS: i32 = 88;
    pub const ENOTEMPTY: i32 = 90;
    pub const ENAMETOOLONG: i32 = 91;

    pub fn by_name(name: &str) -> Option<i32> {
        Some(match name {
            "EPERM" => EPERM,
            "ENOENT" => ENOENT,
            "EINTR" => EINTR,
            "EIO" => EIO,
            "EBADF" => EBADF,
            "EAGAIN" => EAGAIN,
            "ENOMEM" => ENOMEM,
            "EACCES" => EACCES,
            "EEXIST" => EEXIST,
            "ENOTDIR" => ENOTDIR,
            "EISDIR" => EISDIR,
            "EINVAL" => EINVAL,
            "EMFILE" => EMFILE,
            "ENOSPC" => ENOSPC,
            "ESPIPE" => ESPIPE,
            "EROFS" => EROFS,
            "ENOSYS" => ENOSYS,
            "ENOTEMPTY" => ENOTEMPTY,
            "ENAMETOOLONG" => ENAMETOOLONG,
            _ => return None,
        })
    }
}

/// newlib `open()` flags (protocol.md §5.5.2).
pub mod oflag {
    pub const ACCMODE: i32 = 3;
    pub const RDONLY: i32 = 0;
    pub const WRONLY: i32 = 1;
    pub const RDWR: i32 = 2;
    pub const APPEND: i32 = 0x8;
    pub const CREAT: i32 = 0x200;
    pub const TRUNC: i32 = 0x400;
    pub const EXCL: i32 = 0x800;
}

/// A parsed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Isatty {
        fid: i32,
    },
    Fstat {
        fid: i32,
    },
    Close {
        fid: i32,
    },
    Open {
        flags: i32,
        path: String,
    },
    Write {
        fid: u32,
        reply: bool,
        data: Vec<u8>,
    },
    Read {
        fid: i32,
        len: u32,
    },
    Mkdir {
        mode: i32,
        path: String,
    },
    TimeOfDay,
    Clock,
    Time,
    Elapsed,
    TickFreq,
    /// Known op that hosts do not implement (SYSTEM, RENAME, UNLINK, LINK, STAT, LSEEK).
    Unsupported {
        op: u8,
    },
    Unknown {
        op: u8,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RequestError {
    #[error("empty request")]
    Empty,
    #[error("{op}: malformed request ({len} argument bytes)")]
    Malformed { op: &'static str, len: usize },
    #[error("{op}: path is not UTF-8")]
    BadPath { op: &'static str },
}

impl RequestError {
    /// The op byte, when there was one (for the error reply).
    pub fn op(&self, frame: &[u8]) -> Option<u8> {
        frame.first().copied()
    }
}

fn i32_at(a: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(a[at..at + 4].try_into().unwrap())
}

fn u32_at(a: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(a[at..at + 4].try_into().unwrap())
}

pub fn parse_request(frame: &[u8]) -> Result<Request, RequestError> {
    let (&o, a) = frame.split_first().ok_or(RequestError::Empty)?;
    let name = op_name(o).unwrap_or("?");
    let bad = || RequestError::Malformed { op: name, len: a.len() };
    let path = |b: &[u8]| String::from_utf8(b.to_vec()).map_err(|_| RequestError::BadPath { op: name });
    Ok(match o {
        op::ISATTY | op::FSTAT | op::CLOSE => {
            if a.len() != 4 {
                return Err(bad());
            }
            let fid = i32_at(a, 0);
            match o {
                op::ISATTY => Request::Isatty { fid },
                op::FSTAT => Request::Fstat { fid },
                _ => Request::Close { fid },
            }
        }
        op::OPEN | op::MKDIR => {
            if a.len() < 4 {
                return Err(bad());
            }
            let v = i32_at(a, 0);
            let p = path(&a[4..])?;
            if o == op::OPEN {
                Request::Open { flags: v, path: p }
            } else {
                Request::Mkdir { mode: v, path: p }
            }
        }
        op::WRITE => {
            if a.len() < 4 {
                return Err(bad());
            }
            let raw = u32_at(a, 0);
            Request::Write { fid: raw & 0x7FFF_FFFF, reply: raw & WRITE_NO_REPLY == 0, data: a[4..].to_vec() }
        }
        op::READ => {
            if a.len() != 8 {
                return Err(bad());
            }
            Request::Read { fid: i32_at(a, 0), len: u32_at(a, 4) }
        }
        op::TIMEOFDAY | op::SYS_CLOCK | op::SYS_TIME | op::SYS_ELAPSED | op::SYS_TICKFREQ => {
            if !a.is_empty() {
                return Err(bad());
            }
            match o {
                op::TIMEOFDAY => Request::TimeOfDay,
                op::SYS_CLOCK => Request::Clock,
                op::SYS_TIME => Request::Time,
                op::SYS_ELAPSED => Request::Elapsed,
                _ => Request::TickFreq,
            }
        }
        op::SYSTEM | op::RENAME | op::UNLINK | op::LINK | op::STAT | op::LSEEK => Request::Unsupported { op: o },
        _ => Request::Unknown { op: o },
    })
}

/// Size of an op's fixed-size success reply; error replies are zero padded to it.
pub fn fixed_reply_size(o: u8) -> usize {
    if o == op::FSTAT {
        48
    } else {
        8
    }
}

/// Error reply `<i32 -1, i32 errno>`, padded for fixed-size replies (protocol.md §5.5.4).
pub fn error_reply(o: u8, errno: i32) -> Vec<u8> {
    let mut v = Vec::with_capacity(fixed_reply_size(o));
    v.extend_from_slice(&(-1i32).to_le_bytes());
    v.extend_from_slice(&errno.to_le_bytes());
    v.resize(fixed_reply_size(o), 0);
    v
}

/// `<i32 r, i32 errno>` reply.
pub fn reply(r: i32, errno: i32) -> Vec<u8> {
    let mut v = r.to_le_bytes().to_vec();
    v.extend_from_slice(&errno.to_le_bytes());
    v
}

/// READ success reply: `<i32 n, i32 0> ‖ data`.
pub fn read_reply(data: &[u8]) -> Vec<u8> {
    let mut v = reply(data.len() as i32, 0);
    v.extend_from_slice(data);
    v
}

/// FSTAT fields (protocol.md §5.5.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stat {
    pub mode: u32,
    pub ino: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u32,
    pub atime: u32,
    pub mtime: u32,
    pub ctime: u32,
}

pub fn fstat_reply(s: &Stat) -> Vec<u8> {
    let mut v = reply(0, 0);
    for x in [s.mode, s.ino, 0, s.nlink, s.uid, s.gid, s.size, s.atime, s.mtime, s.ctime] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v
}

/// Inputs for the time ops: a monotonic counter and wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSample {
    pub ticks: u64,
    pub freq: u64,
    /// Seconds since the Unix epoch.
    pub wall: f64,
}

/// Reply for a time op (protocol.md §5.5.3).
pub fn time_reply(o: u8, t: &TimeSample) -> Option<Vec<u8>> {
    Some(match o {
        op::SYS_CLOCK => (((t.ticks as u128 * 100) / t.freq as u128) as u32).to_le_bytes().to_vec(),
        op::SYS_TIME => (t.wall.floor() as i64).to_le_bytes().to_vec(),
        op::SYS_ELAPSED => t.ticks.to_le_bytes().to_vec(),
        op::SYS_TICKFREQ => t.freq.to_le_bytes().to_vec(),
        op::TIMEOFDAY => {
            let sec = t.wall.floor();
            let usec = (((t.wall - sec) * 1e6 + 0.5) as u32).min(999_999);
            let mut v = (sec as u64).to_le_bytes().to_vec();
            v.extend_from_slice(&usec.to_le_bytes());
            v
        }
        _ => return None,
    })
}
