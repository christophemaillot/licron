use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::time::SystemTime;

use crate::cron::find_next_run;
use crate::model::{
    CronSpec, HEADER_SCAN_LINES, IgnoredJob, LoadedJob, ParsedHeader, RuntimeConfig, TimeMode,
};

pub fn scan_jobs(
    jobs_dir: &Path,
    cfg: &RuntimeConfig,
    time_mode: TimeMode,
) -> (Vec<LoadedJob>, Vec<IgnoredJob>) {
    let mut loaded = Vec::new();
    let mut ignored = Vec::new();

    let entries = match fs::read_dir(jobs_dir) {
        Ok(entries) => entries,
        Err(err) => {
            ignored.push(IgnoredJob {
                path: jobs_dir.to_path_buf(),
                reason: format!("failed to read directory: {err}"),
            });
            return (loaded, ignored);
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();

        if let Err(reason) = validate_job_candidate(&path, &file_name) {
            ignored.push(IgnoredJob { path, reason });
            continue;
        }

        let parsed = match parse_header_metadata(&path) {
            Ok(meta) => meta,
            Err(reason) => {
                ignored.push(IgnoredJob { path, reason });
                continue;
            }
        };

        let cron = match CronSpec::parse(&parsed.cron_expr) {
            Ok(spec) => spec,
            Err(err) => {
                ignored.push(IgnoredJob {
                    path,
                    reason: format!("invalid cron expression '{}': {err}", parsed.cron_expr),
                });
                continue;
            }
        };

        let selected_timeout = parsed.timeout_secs.unwrap_or(cfg.default_timeout_secs);
        let timeout_secs = if selected_timeout > cfg.max_timeout_secs {
            eprintln!(
                "warn: {} timeout capped from {}s to {}s",
                path.display(),
                selected_timeout,
                cfg.max_timeout_secs
            );
            cfg.max_timeout_secs
        } else {
            selected_timeout
        };

        let next_run = find_next_run(&cron, SystemTime::now(), time_mode);

        loaded.push(LoadedJob {
            path,
            cron_expr: parsed.cron_expr,
            cron,
            timeout_secs,
            next_run,
        });
    }

    loaded.sort_by(|a, b| a.path.cmp(&b.path));
    ignored.sort_by(|a, b| a.path.cmp(&b.path));

    (loaded, ignored)
}

pub fn validate_job_candidate(path: &Path, file_name: &OsStr) -> Result<(), String> {
    let file_name = file_name
        .to_str()
        .ok_or_else(|| "file name is not valid UTF-8".to_string())?;

    if file_name.starts_with('.') {
        return Err("hidden file".to_string());
    }

    if !is_valid_job_name(file_name) {
        return Err("invalid name: expected ^[A-Za-z0-9_-]+(\\.[A-Za-z0-9]+)?$".to_string());
    }

    let metadata =
        fs::symlink_metadata(path).map_err(|e| format!("failed to read file metadata: {e}"))?;

    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err("symlink is not allowed".to_string());
    }
    if !file_type.is_file() {
        return Err("not a regular file".to_string());
    }
    if file_type.is_socket()
        || file_type.is_fifo()
        || file_type.is_char_device()
        || file_type.is_block_device()
    {
        return Err("unsupported file type".to_string());
    }

    let mode = metadata.permissions().mode();
    if mode & 0o111 == 0 {
        return Err("not executable".to_string());
    }

    Ok(())
}

pub fn is_valid_job_name(name: &str) -> bool {
    let mut parts = name.split('.');
    let base = parts.next().unwrap_or("");
    if base.is_empty()
        || !base
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return false;
    }

    match parts.next() {
        None => true,
        Some(ext) => {
            if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                return false;
            }
            parts.next().is_none()
        }
    }
}

pub fn parse_header_metadata(path: &Path) -> Result<ParsedHeader, String> {
    let file = fs::File::open(path).map_err(|e| format!("failed to open file: {e}"))?;
    let reader = BufReader::new(file);

    let mut cron_expr: Option<String> = None;
    let mut timeout_secs: Option<u64> = None;

    for (idx, line) in reader.lines().take(HEADER_SCAN_LINES).enumerate() {
        let line = line.map_err(|e| format!("failed to read line {}: {e}", idx + 1))?;
        let trimmed = line.trim();

        if let Some(value) = parse_prefixed_value(trimmed, "licron") {
            if cron_expr.is_none() {
                cron_expr = Some(value.to_string());
            } else {
                eprintln!(
                    "warn: {} duplicate 'licron' metadata at line {} ignored",
                    path.display(),
                    idx + 1
                );
            }
            continue;
        }

        if let Some(value) = parse_prefixed_value(trimmed, "licron-timeout") {
            match parse_duration_secs(value) {
                Ok(seconds) => {
                    if timeout_secs.is_none() {
                        timeout_secs = Some(seconds);
                    } else {
                        eprintln!(
                            "warn: {} duplicate 'licron-timeout' metadata at line {} ignored",
                            path.display(),
                            idx + 1
                        );
                    }
                }
                Err(err) => {
                    return Err(format!("invalid licron-timeout '{}': {err}", value));
                }
            }
        }
    }

    let cron_expr = cron_expr.ok_or_else(|| "missing 'licron' metadata".to_string())?;

    Ok(ParsedHeader {
        cron_expr,
        timeout_secs,
    })
}

pub fn parse_prefixed_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let content = line.strip_prefix('#')?.trim_start();
    let (left, right) = content.split_once(':')?;
    if left.trim() != key {
        return None;
    }
    let value = right.trim();
    if value.is_empty() {
        return None;
    }
    Some(value)
}

pub fn parse_duration_secs(raw: &str) -> Result<u64, String> {
    let raw = raw.trim();
    if raw.len() < 2 {
        return Err("duration must look like <int>s|m|h".to_string());
    }

    let (num, unit) = raw.split_at(raw.len() - 1);
    let value: u64 = num
        .parse()
        .map_err(|_| "duration value must be an integer".to_string())?;

    let mul = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        _ => return Err("duration unit must be one of: s, m, h".to_string()),
    };

    value
        .checked_mul(mul)
        .ok_or_else(|| "duration is too large".to_string())
}
