use std::fs;
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::model::{DateParts, TimeMode};

const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;
const LOCK_UN: i32 = 8;

type TimeT = i64;
type PidT = i32;
type SigHandler = extern "C" fn(i32);
const SIGTERM: i32 = 15;
const SIGINT: i32 = 2;
const SIG_ERR: usize = usize::MAX;

static TERMINATE_REQUESTED: AtomicBool = AtomicBool::new(false);

pub struct InstanceLock {
    file: fs::File,
    lock_path: std::path::PathBuf,
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let fd = self.file.as_raw_fd();
        unsafe {
            let _ = flock(fd, LOCK_UN);
        }
        eprintln!(
            "info: instance lock released path={}",
            self.lock_path.display()
        );
    }
}

pub fn acquire_instance_lock(jobs_dir: &std::path::Path) -> Result<InstanceLock, String> {
    let lock_path = jobs_dir.join(".licron.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("failed to open lock file {}: {e}", lock_path.display()))?;

    let fd = file.as_raw_fd();
    let rc = unsafe { flock(fd, LOCK_EX | LOCK_NB) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        return Err(format!(
            "another licron instance appears active for jobs_dir={} (lock={}) ({})",
            jobs_dir.display(),
            lock_path.display(),
            err
        ));
    }

    eprintln!("info: instance lock acquired path={}", lock_path.display());
    Ok(InstanceLock { file, lock_path })
}

extern "C" fn handle_termination_signal(_sig: i32) {
    TERMINATE_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn install_signal_handlers() -> Result<(), String> {
    let sigint_res = unsafe { signal(SIGINT, handle_termination_signal) };
    let sigterm_res = unsafe { signal(SIGTERM, handle_termination_signal) };
    if sigint_res == SIG_ERR || sigterm_res == SIG_ERR {
        return Err(format!(
            "signal registration failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

pub fn termination_requested() -> bool {
    TERMINATE_REQUESTED.load(Ordering::SeqCst)
}

pub fn send_sigterm(pid: u32) -> bool {
    let pid = pid as PidT;
    unsafe { kill(pid, SIGTERM) == 0 }
}

pub fn resolve_time_mode(timezone: &Option<String>) -> Result<TimeMode, String> {
    match timezone.as_deref() {
        None => Ok(TimeMode::Local),
        Some("local") | Some("LOCAL") => Ok(TimeMode::Local),
        Some("UTC") | Some("utc") => Ok(TimeMode::Utc),
        Some(other) => {
            if !is_valid_iana_timezone(other) {
                return Err(format!(
                    "invalid timezone '{}': expected 'local', 'UTC', or a valid IANA zone name",
                    other
                ));
            }
            set_process_timezone(other)?;
            Ok(TimeMode::Named)
        }
    }
}

pub fn unix_secs(time: SystemTime) -> Option<i64> {
    let d = time.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(d.as_secs()).ok()
}

pub fn add_seconds(time: SystemTime, seconds: u64) -> Option<SystemTime> {
    time.checked_add(Duration::from_secs(seconds))
}

pub fn floor_to_minute(time: SystemTime) -> SystemTime {
    if let Ok(d) = time.duration_since(UNIX_EPOCH) {
        let secs = d.as_secs() - (d.as_secs() % 60);
        UNIX_EPOCH + Duration::from_secs(secs)
    } else {
        time
    }
}

pub fn format_system_time(time: SystemTime, mode: TimeMode) -> Option<String> {
    let unix = unix_secs(time)?;
    let dt = date_parts_from_unix(unix, mode)?;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        dt.year, dt.month, dt.day, dt.hour, dt.minute
    ))
}

pub fn date_parts_from_unix(unix_secs: i64, mode: TimeMode) -> Option<DateParts> {
    let mut tm = Tm::default();
    let t = unix_secs as TimeT;

    let ptr = unsafe {
        match mode {
            TimeMode::Local | TimeMode::Named => {
                localtime_r(&t as *const TimeT, &mut tm as *mut Tm)
            }
            TimeMode::Utc => gmtime_r(&t as *const TimeT, &mut tm as *mut Tm),
        }
    };

    if ptr.is_null() {
        return None;
    }

    Some(DateParts {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        wday: tm.tm_wday as u32,
    })
}

fn is_valid_iana_timezone(name: &str) -> bool {
    if name.is_empty() || name.starts_with('/') || name.contains("..") {
        return false;
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
    {
        return false;
    }
    let zone_path = Path::new("/usr/share/zoneinfo").join(name);
    zone_path.is_file()
}

fn set_process_timezone(name: &str) -> Result<(), String> {
    let key = std::ffi::CString::new("TZ").map_err(|_| "failed to build TZ key".to_string())?;
    let value = std::ffi::CString::new(name)
        .map_err(|_| "timezone contains invalid NUL byte".to_string())?;
    let rc = unsafe { setenv(key.as_ptr(), value.as_ptr(), 1) };
    if rc != 0 {
        return Err(format!(
            "failed to set TZ='{}': {}",
            name,
            std::io::Error::last_os_error()
        ));
    }
    unsafe {
        tzset();
    }
    Ok(())
}

#[repr(C)]
#[derive(Default, Copy, Clone)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const u8,
}

unsafe extern "C" {
    fn localtime_r(timep: *const TimeT, result: *mut Tm) -> *mut Tm;
    fn gmtime_r(timep: *const TimeT, result: *mut Tm) -> *mut Tm;
    fn kill(pid: PidT, sig: i32) -> i32;
    fn flock(fd: i32, operation: i32) -> i32;
    fn signal(sig: i32, handler: SigHandler) -> usize;
    fn setenv(name: *const i8, value: *const i8, overwrite: i32) -> i32;
    fn tzset();
}
