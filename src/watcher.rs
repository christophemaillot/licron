use std::collections::HashMap;
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

#[cfg(target_os = "linux")]
use crate::model::INOTIFY_DEBOUNCE_MS;
use crate::model::RELOAD_POLL_INTERVAL_SECS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirSnapshot {
    entries: HashMap<PathBuf, DirEntrySig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirEntrySig {
    mode: u32,
    len: u64,
    modified_secs: Option<u64>,
    is_dir: bool,
    is_file: bool,
    is_symlink: bool,
}

pub trait WatcherBackend {
    fn poll_changed(&mut self, jobs_dir: &Path) -> Result<bool, String>;
}

pub struct PollWatcher {
    interval: Duration,
    last_check: Instant,
    last_snapshot: Option<DirSnapshot>,
}

impl PollWatcher {
    fn new(interval: Duration, jobs_dir: &Path) -> Self {
        Self {
            interval,
            last_check: Instant::now(),
            last_snapshot: snapshot_jobs_dir(jobs_dir).ok(),
        }
    }
}

impl WatcherBackend for PollWatcher {
    fn poll_changed(&mut self, jobs_dir: &Path) -> Result<bool, String> {
        if self.last_check.elapsed() < self.interval {
            return Ok(false);
        }
        self.last_check = Instant::now();
        let snapshot = snapshot_jobs_dir(jobs_dir)?;
        let changed = match &self.last_snapshot {
            Some(prev) => prev != &snapshot,
            None => true,
        };
        self.last_snapshot = Some(snapshot);
        Ok(changed)
    }
}

#[cfg(target_os = "linux")]
pub struct InotifyWatcher {
    fd: i32,
    pending_since: Option<Instant>,
    debounce: Duration,
}

#[cfg(target_os = "linux")]
impl InotifyWatcher {
    fn new(jobs_dir: &Path, debounce: Duration) -> Result<Self, String> {
        let fd = unsafe { inotify_init1(IN_NONBLOCK | IN_CLOEXEC) };
        if fd < 0 {
            return Err(format!(
                "inotify_init1 failed: errno={}",
                read_errno().unwrap_or(-1)
            ));
        }

        let path_c = std::ffi::CString::new(jobs_dir.as_os_str().as_bytes().to_vec())
            .map_err(|_| "jobs_dir contains interior NUL byte".to_string())?;
        let wd = unsafe { inotify_add_watch(fd, path_c.as_ptr(), INOTIFY_WATCH_MASK) };
        if wd < 0 {
            unsafe {
                close(fd);
            }
            return Err(format!(
                "inotify_add_watch failed: errno={}",
                read_errno().unwrap_or(-1)
            ));
        }

        Ok(Self {
            fd,
            pending_since: None,
            debounce,
        })
    }
}

#[cfg(target_os = "linux")]
impl Drop for InotifyWatcher {
    fn drop(&mut self) {
        unsafe {
            close(self.fd);
        }
    }
}

#[cfg(target_os = "linux")]
impl WatcherBackend for InotifyWatcher {
    fn poll_changed(&mut self, _jobs_dir: &Path) -> Result<bool, String> {
        let mut buf = [0u8; 8192];
        let mut saw_event = false;
        loop {
            let n = unsafe { read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                saw_event = true;
                continue;
            }
            if n == 0 {
                break;
            }
            let errno = read_errno().unwrap_or(-1);
            if errno == EAGAIN || errno == EWOULDBLOCK {
                break;
            }
            return Err(format!("inotify read failed: errno={errno}"));
        }

        if saw_event {
            self.pending_since = Some(Instant::now());
            return Ok(false);
        }

        if let Some(since) = self.pending_since {
            if since.elapsed() >= self.debounce {
                self.pending_since = None;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

pub fn create_watcher(jobs_dir: &Path) -> Box<dyn WatcherBackend> {
    #[cfg(target_os = "linux")]
    {
        match InotifyWatcher::new(jobs_dir, Duration::from_millis(INOTIFY_DEBOUNCE_MS)) {
            Ok(w) => {
                eprintln!(
                    "info: watcher backend=inotify debounce_ms={}",
                    INOTIFY_DEBOUNCE_MS
                );
                return Box::new(w);
            }
            Err(err) => {
                eprintln!("warn: inotify unavailable, falling back to polling: {err}");
            }
        }
    }

    eprintln!(
        "info: watcher backend=poll interval_s={}",
        RELOAD_POLL_INTERVAL_SECS
    );
    Box::new(PollWatcher::new(
        Duration::from_secs(RELOAD_POLL_INTERVAL_SECS),
        jobs_dir,
    ))
}

fn snapshot_jobs_dir(jobs_dir: &Path) -> Result<DirSnapshot, String> {
    let mut entries = HashMap::new();
    let dir_iter = fs::read_dir(jobs_dir).map_err(|e| format!("read_dir failed: {e}"))?;

    for entry in dir_iter {
        let entry = entry.map_err(|e| format!("read_dir entry failed: {e}"))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|e| format!("metadata read failed for {}: {e}", path.display()))?;
        let modified_secs = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        entries.insert(
            path,
            DirEntrySig {
                mode: metadata.permissions().mode(),
                len: metadata.len(),
                modified_secs,
                is_dir: metadata.is_dir(),
                is_file: metadata.is_file(),
                is_symlink: metadata.file_type().is_symlink(),
            },
        );
    }

    Ok(DirSnapshot { entries })
}

#[cfg(target_os = "linux")]
type SizeT = usize;
#[cfg(target_os = "linux")]
const IN_NONBLOCK: i32 = 0o0004000;
#[cfg(target_os = "linux")]
const IN_CLOEXEC: i32 = 0o2000000;
#[cfg(target_os = "linux")]
const EAGAIN: i32 = 11;
#[cfg(target_os = "linux")]
const EWOULDBLOCK: i32 = 11;
#[cfg(target_os = "linux")]
const INOTIFY_WATCH_MASK: u32 = IN_CREATE
    | IN_DELETE
    | IN_MODIFY
    | IN_ATTRIB
    | IN_MOVED_FROM
    | IN_MOVED_TO
    | IN_CLOSE_WRITE
    | IN_DELETE_SELF
    | IN_MOVE_SELF;
#[cfg(target_os = "linux")]
const IN_CREATE: u32 = 0x0000_0100;
#[cfg(target_os = "linux")]
const IN_DELETE: u32 = 0x0000_0200;
#[cfg(target_os = "linux")]
const IN_MODIFY: u32 = 0x0000_0002;
#[cfg(target_os = "linux")]
const IN_ATTRIB: u32 = 0x0000_0004;
#[cfg(target_os = "linux")]
const IN_MOVED_FROM: u32 = 0x0000_0040;
#[cfg(target_os = "linux")]
const IN_MOVED_TO: u32 = 0x0000_0080;
#[cfg(target_os = "linux")]
const IN_CLOSE_WRITE: u32 = 0x0000_0008;
#[cfg(target_os = "linux")]
const IN_DELETE_SELF: u32 = 0x0000_0400;
#[cfg(target_os = "linux")]
const IN_MOVE_SELF: u32 = 0x0000_0800;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn inotify_init1(flags: i32) -> i32;
    fn inotify_add_watch(fd: i32, pathname: *const i8, mask: u32) -> i32;
    fn read(fd: i32, buf: *mut core::ffi::c_void, count: SizeT) -> isize;
    fn close(fd: i32) -> i32;
    fn __errno_location() -> *mut i32;
}

#[cfg(target_os = "linux")]
fn read_errno() -> Option<i32> {
    let ptr = unsafe { __errno_location() };
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { *ptr })
    }
}
