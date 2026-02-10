# LICRON

A lightweight Linux daemon that runs executable scripts from a directory using cron schedules declared inside each script header.

LICRON is designed for simple, file-based job orchestration:
- Drop executable files in a jobs directory.
- Declare schedule metadata at the top of each file.
- LICRON watches the directory and executes jobs on schedule.

## Features

- Cron-like scheduling from file headers (`# licron: ...`)
- Live directory watch (create/modify/delete/chmod/rename)
- Strict and predictable filename policy
- Overlap protection per job (skip if already running)
- Global and per-job execution timeouts
- Structured logs for parsing, scheduling, and execution lifecycle
- No database required; state is in-memory and rebuilt from files

## Requirements

- Linux
- Executable scripts with valid shebangs
- Read access to jobs directory
- Execute permission (`+x`) on job files

## Installation

### Install from GitHub (standard Rust way)

```bash
cargo install --git https://github.com/christophemaillot/licron --locked
```

This compiles and installs `licron` into Cargo's bin directory (typically `~/.cargo/bin`).

### Install from crates.io (when published)

```bash
cargo install licron --locked
```

### Build from source

```bash
cargo build --release
```

Binary path:

```bash
./target/release/licron
```

### Install locally

```bash
cargo install --path .
```

## Quick Start

1. Create a jobs directory:

```bash
mkdir -p /opt/licron/jobs
```

2. Add a job file:

```bash
cat >/opt/licron/jobs/01-backup.sh <<'SH'
#!/usr/bin/env bash
# licron: 5 1 * * *
# licron-timeout: 10m

echo "running daily backup"
SH
chmod +x /opt/licron/jobs/01-backup.sh
```

3. Start LICRON:

```bash
licron /opt/licron/jobs
```

## Example Job Sets

- Valid sample jobs: `examples/jobs`
- Intentionally invalid parsing/name samples: `examples/invalid-jobs`

These are useful for validating both execution flow and ignore/error logging behavior.

## CLI

```text
licron <jobs_dir> [options]
```

### Required

- `<jobs_dir>`: directory to scan/watch for jobs (non-recursive)

### Options

- `--timezone <TZ>`
  - Global timezone used for schedule evaluation
  - Default: system local timezone
- `--max-parallel <N>`
  - Maximum concurrently running jobs
  - Default: `4`
- `--default-timeout <duration>`
  - Default runtime timeout when a job has no `licron-timeout`
  - Default: `300s`
- `--max-timeout <duration>`
  - Upper bound for any effective timeout
  - Default: `3600s`
- `--dry-run`
  - Parse jobs and print schedules without executing anything
- `--oneshot`
  - Single scan/parse/schedule report, then exit

Duration format:
- `<int>s`, `<int>m`, `<int>h`
- Examples: `30s`, `5m`, `1h`

## Job File Rules

A file is accepted as a job only if all checks pass:

1. Regular file
2. Executable (`+x`)
3. Name matches:

```regex
^[A-Za-z0-9_-]+(\.[A-Za-z0-9]+)?$
```

Valid examples:
- `01-backup`
- `cleanup.py`
- `rotate.zsh`

Invalid examples:
- `.hidden`
- `job.tar.gz`
- `backup job.sh`
- `a+b.py`

## Header Metadata

LICRON reads the first 50 lines of each candidate file and parses metadata comments.

Required schedule metadata:

```text
# licron: <cron_expr>
```

Optional timeout metadata:

```text
# licron-timeout: <duration>
```

Example:

```bash
#!/usr/bin/env bash
# licron: */15 * * * *
# licron-timeout: 90s

echo "quarter-hour task"
```

### Metadata behavior

- Missing/invalid `licron` -> file ignored and logged
- Invalid `licron-timeout` -> file ignored and logged
- Multiple metadata lines of same key -> first valid one is used; duplicates logged

## Cron Support

LICRON supports standard 5-field cron:

```text
minute hour day_of_month month day_of_week
```

Supported:
- `*`
- numeric literals
- lists (`,`) 
- ranges (`-`)
- steps (`/`)

Not supported:
- `@daily`, `@hourly`, etc.
- seconds field
- `L`, `W`, `#`, `?`

## Scheduling and Concurrency

- Jobs run by schedule time only (no lexical run-parts ordering)
- If a trigger fires while the same job is still running, LICRON skips that run and logs `job_skipped_running`
- Global concurrency is limited by `--max-parallel`
- Single-instance assumption per jobs directory in v1

## Timeout Behavior

Effective timeout resolution:

1. Use `# licron-timeout` if present and valid
2. Otherwise use `--default-timeout`
3. If above `--max-timeout`, cap to `--max-timeout` and log warning

When timeout is reached:

1. Send `SIGTERM`
2. Wait grace period (10s)
3. Send `SIGKILL` if still running

## Execution Environment

- Jobs are executed directly (no shell wrapping)
- Shebang decides interpreter
- Working directory: jobs directory
- Extra env vars provided to each job:
  - `LICRON_JOB_NAME`
  - `LICRON_JOB_PATH`
  - `LICRON_SCHEDULE`

## Logging

LICRON emits structured events such as:

- `daemon_start`, `daemon_stop`
- `scan_started`, `scan_completed`
- `job_loaded`, `job_updated`, `job_unloaded`
- `job_ignored`
- `job_triggered`
- `job_skipped_running`
- `job_timeout`
- `job_kill_escalation`
- `job_exit`
- `job_exec_error`

## Running with systemd

Example unit file:

```ini
[Unit]
Description=LICRON Job Scheduler
After=network.target

[Service]
Type=simple
User=licron
Group=licron
ExecStart=/usr/local/bin/licron /opt/licron/jobs --max-parallel 4 --default-timeout 300s --max-timeout 3600s
Restart=always
RestartSec=2

[Install]
WantedBy=multi-user.target
```

Install and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now licron
sudo systemctl status licron
```

## Operational Notes

- LICRON does not replay missed runs after restart/downtime
- A periodic full rescan complements file watching for robustness
- Per-file parse errors are non-fatal to daemon operation

## Troubleshooting

- Job not running:
  - Check file is executable (`chmod +x`)
  - Verify filename matches policy
  - Verify `# licron:` line is in first 50 lines
  - Validate cron expression and timeout format
- Job skipped unexpectedly:
  - Check for `job_skipped_running` logs (previous invocation still active)
- Job killed:
  - Check `job_timeout` / `job_kill_escalation` logs

## License

TBD
