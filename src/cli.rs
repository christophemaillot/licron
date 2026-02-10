use std::env;
use std::fs;
use std::path::Path;

use crate::model::{
    Cli, DEFAULT_MAX_PARALLEL, DEFAULT_MAX_TIMEOUT_SECS, DEFAULT_TIMEOUT_SECS, RuntimeConfig,
};
use crate::scanner::parse_duration_secs;

pub fn parse_cli(args: Vec<String>) -> Result<Cli, String> {
    if args.len() == 1 {
        return Err("missing jobs_dir argument".to_string());
    }

    let mut timezone = None;
    let mut max_parallel = DEFAULT_MAX_PARALLEL;
    let mut default_timeout = format!("{}s", DEFAULT_TIMEOUT_SECS);
    let mut max_timeout = format!("{}s", DEFAULT_MAX_TIMEOUT_SECS);
    let mut dry_run = false;
    let mut oneshot = false;
    let mut jobs_dir: Option<std::path::PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("licron {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--timezone" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--timezone requires a value".to_string())?;
                timezone = Some(value.clone());
            }
            "--max-parallel" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--max-parallel requires a value".to_string())?;
                max_parallel = value
                    .parse::<usize>()
                    .map_err(|_| "--max-parallel must be an integer".to_string())?;
                if max_parallel == 0 {
                    return Err("--max-parallel must be greater than zero".to_string());
                }
            }
            "--default-timeout" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--default-timeout requires a value".to_string())?;
                default_timeout = value.clone();
            }
            "--max-timeout" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--max-timeout requires a value".to_string())?;
                max_timeout = value.clone();
            }
            "--dry-run" => dry_run = true,
            "--oneshot" => oneshot = true,
            other if other.starts_with("--") => {
                return Err(format!("unknown flag: {other}"));
            }
            _ => {
                if jobs_dir.is_some() {
                    return Err("multiple jobs_dir arguments provided".to_string());
                }
                jobs_dir = Some(std::path::PathBuf::from(arg));
            }
        }
        i += 1;
    }

    let jobs_dir = jobs_dir.ok_or_else(|| "missing jobs_dir argument".to_string())?;

    Ok(Cli {
        jobs_dir,
        timezone,
        max_parallel,
        default_timeout,
        max_timeout,
        dry_run,
        oneshot,
    })
}

pub fn print_usage() {
    println!(
        "Usage: licron <jobs_dir> [--timezone <TZ>] [--max-parallel <N>] [--default-timeout <DUR>] [--max-timeout <DUR>] [--dry-run] [--oneshot]"
    );
}

pub fn validate_jobs_dir(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|e| format!("cannot read jobs directory: {e}"))?;
    if !metadata.is_dir() {
        return Err(format!("'{}' is not a directory", path.display()));
    }
    Ok(())
}

pub fn build_runtime_config(cli: &Cli) -> Result<RuntimeConfig, String> {
    let default_timeout_secs =
        parse_duration_secs(&cli.default_timeout).map_err(|e| format!("--default-timeout: {e}"))?;
    let max_timeout_secs =
        parse_duration_secs(&cli.max_timeout).map_err(|e| format!("--max-timeout: {e}"))?;

    if max_timeout_secs == 0 {
        return Err("--max-timeout must be greater than zero".to_string());
    }

    Ok(RuntimeConfig {
        default_timeout_secs,
        max_timeout_secs,
    })
}
