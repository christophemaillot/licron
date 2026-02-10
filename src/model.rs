use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Child;
use std::time::{Instant, SystemTime};

pub const HEADER_SCAN_LINES: usize = 50;
pub const DEFAULT_MAX_PARALLEL: usize = 4;
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;
pub const DEFAULT_MAX_TIMEOUT_SECS: u64 = 3600;
pub const DEFAULT_TIMEOUT_GRACE_SECS: u64 = 10;
pub const NEXT_RUN_SCAN_MINUTES: u64 = 366 * 24 * 60;
pub const RELOAD_POLL_INTERVAL_SECS: u64 = 2;
pub const FULL_RESCAN_INTERVAL_SECS: u64 = 60;
#[cfg(target_os = "linux")]
pub const INOTIFY_DEBOUNCE_MS: u64 = 250;

#[derive(Debug)]
pub struct Cli {
    pub jobs_dir: PathBuf,
    pub timezone: Option<String>,
    pub max_parallel: usize,
    pub default_timeout: String,
    pub max_timeout: String,
    pub dry_run: bool,
    pub oneshot: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeConfig {
    pub default_timeout_secs: u64,
    pub max_timeout_secs: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum TimeMode {
    Local,
    Utc,
    Named,
}

#[derive(Debug)]
pub struct LoadedJob {
    pub path: PathBuf,
    pub cron_expr: String,
    pub cron: CronSpec,
    pub timeout_secs: u64,
    pub next_run: Option<SystemTime>,
}

#[derive(Debug)]
pub struct IgnoredJob {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug)]
pub struct ParsedHeader {
    pub cron_expr: String,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CronSpec {
    pub minutes: CronField,
    pub hours: CronField,
    pub dom: CronField,
    pub months: CronField,
    pub dow: CronField,
    pub dom_any: bool,
    pub dow_any: bool,
}

#[derive(Debug, Clone)]
pub struct CronField {
    pub allowed: HashSet<u32>,
    pub any: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct DateParts {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub wday: u32,
}

#[derive(Debug)]
pub struct RunningProc {
    pub child: Child,
    pub started_at: Instant,
    pub timeout_at: Instant,
    pub term_sent_at: Option<Instant>,
}

#[derive(Debug)]
pub struct JobRuntime {
    pub job: LoadedJob,
    pub running: Option<RunningProc>,
    pub schedule_enabled: bool,
}
