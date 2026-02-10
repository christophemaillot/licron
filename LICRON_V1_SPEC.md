# LICRON v1 Specification

## 1. Overview

`licron` is a Linux daemon that runs executable files from a configured directory according to per-file cron metadata declared in script headers.

Design goals for v1:
- Minimal, deterministic behavior.
- Cron-like scheduling with low operational surprise.
- Simple deployment: one daemon, one watched directory.
- Clear logs for all scheduling and parsing decisions.

Out of scope for v1:
- Distributed scheduling.
- Persistent state database.
- Catch-up/replay of missed runs after downtime.
- Multi-directory configuration.
- Per-job timezone.

---

## 2. Runtime Model

- Platform: Linux only.
- Process type: long-running daemon.
- Config input: directory path passed as CLI argument.
- Time basis: daemon-local timezone (default system local time).
- Scheduling policy when multiple jobs are due at once: **pure schedule time** (no lexical run-parts ordering requirement).
- Overlap policy for a single job: **skip if previous run is still active, and log skip event**.

---

## 3. CLI Contract (v1)

### 3.1 Required argument
- `licron <jobs_dir>`

`jobs_dir` must exist and be a readable directory.

### 3.2 Optional flags
- `--timezone <TZ>`
  - Default: system local timezone.
  - Applies globally to all jobs.
- `--max-parallel <N>`
  - Max concurrently running child processes.
  - Default: implementation-defined sane default (recommend 4).
- `--default-timeout <duration>`
  - Default per-job runtime timeout when no header override is present.
  - Recommended default: `300s`.
- `--max-timeout <duration>`
  - Upper bound for any effective job timeout (including header overrides).
  - Recommended default: `3600s`.
- `--dry-run`
  - Scan/parse and print discovered jobs + next run times; do not daemonize or execute jobs.
- `--oneshot`
  - Perform one scan and exit after reporting parse status and schedule calculation.

If both `--dry-run` and `--oneshot` are set, behavior is equivalent to one non-executing scan.

---

## 4. Job Discovery

The daemon scans a single directory (non-recursive) and considers a file a candidate job only if all are true:
- Regular file.
- Executable by daemon user (`+x`).
- Filename matches run-parts naming policy.

### 4.1 run-parts naming policy (v1)
To avoid ambiguity, v1 uses a strict allowlist with a single optional extension:
- Base name allowed chars: `A-Z`, `a-z`, `0-9`, `_`, `-`.
- Optional suffix: exactly one extension matching `.[A-Za-z0-9]+` (for example `.sh`, `.py`, `.zsh`).
- No path separators.
- No leading dot.
- No spaces.
- No other punctuation.

Examples:
- Valid: `01-backup`, `db_cleanup`, `Z99_rotate-logs`, `01-backup.sh`, `cleanup.py`, `rotate.zsh`
- Invalid: `.hidden`, `backup job`, `a+b`, `a..sh`, `job.tar.gz`, `name.`

Reference pattern (implementation guidance): `^[A-Za-z0-9_-]+(\\.[A-Za-z0-9]+)?$`

Note: This is still stricter than classic run-parts while supporting common script extensions.

---

## 5. Header Metadata Format

For each candidate file, `licron` reads the first `N` lines (default recommendation: `N=50`) and searches for one metadata directive:

- Shell-comment form:
  - `# licron: <cron_expr>`
  - `# licron-timeout: <duration>` (optional)

Directive parsing rules:
- Match key name exactly: `licron`.
- Separator: `:`.
- Surrounding whitespace allowed.
- First valid directive in first `N` lines wins.
- Additional directives in same file are ignored and logged as duplicate metadata.
- Timeout key `licron-timeout` is optional; if present, parse as duration (`<int><unit>`) where unit is one of `s`, `m`, `h`.

Example:
```sh
#!/bin/bash
#
# licron: 5 1 * * *
# licron-timeout: 10m
#
```

### 5.1 Missing or invalid metadata
- If no valid `licron` directive is found: file is ignored and logged.
- If directive exists but cron expression is invalid: file is ignored and logged.
- Invalid file must not fail daemon startup.
- If `licron-timeout` is present but invalid, file is ignored and logged.

(Per product decision: **ignore file and log**.)

---

## 6. Cron Expression Semantics

v1 supports standard 5-field cron format only:
- `minute hour day_of_month month day_of_week`

Supported tokens (v1):
- `*`
- Numeric literal in field range
- Comma lists
- Ranges (`a-b`)
- Steps (`*/n`, `a-b/n`)

Not supported in v1:
- `@hourly`, `@daily`, etc.
- Seconds field.
- Extended cron nicknames (`L`, `W`, `#`, `?`).

Field ranges:
- minute: `0-59`
- hour: `0-23`
- day_of_month: `1-31`
- month: `1-12`
- day_of_week: `0-7` (`0` and `7` both Sunday)

DOM/DOW semantics should follow standard cron OR behavior.

---

## 7. Scheduling Engine

In-memory registry per job:
- file path
- parsed schedule
- next fire time
- running state
- current process ID (when running)
- last start/finish timestamps (for logs/diagnostics)

Behavior:
1. On startup, full scan + parse + schedule next run for valid jobs.
2. Main loop sleeps until nearest next run or file-watch update.
3. On wake, collect due jobs (`next_run <= now`) and attempt execution.
4. Immediately compute each executed/skipped job’s subsequent `next_run`.

### 7.1 Time anomalies
- DST/clock changes are evaluated against current wall clock at compute time.
- No backfill of missed times after long pause/restart.

### 7.2 Concurrency and overlap detection

v1 detection mechanism:
- Overlap detection is in-memory per job ID within a single daemon process.
- If a trigger occurs while the same job is marked running, skip run and emit `job_skipped_running`.
- This model assumes one active `licron` instance per `jobs_dir`.

v1 startup guard:
- Daemon should attempt a best-effort singleton guard per `jobs_dir` and log a warning or fail fast when another instance is detected (implementation choice documented by the binary).

v1.1 hardening path (non-v1 requirement):
- Add per-job lock files using non-blocking `flock` (for example under `jobs_dir/.licron-locks/`) to prevent overlaps across multiple daemon instances or restarts.
- Keep in-memory running-state checks for fast-path behavior and diagnostics.

---

## 8. File Watch + Reload

v1 watches `jobs_dir` for:
- create
- modify
- delete
- chmod/attribute changes
- rename into/out of directory

Reload behavior:
- Debounce burst events (recommend 100-500ms window).
- Re-validate changed candidates.
- Add/update/remove jobs in registry atomically.

Robustness fallback:
- Periodic full rescan (recommend every 60s) to recover from missed watcher events.

---

## 9. Execution Model

Execution contract:
- Spawn executable file directly (respecting shebang), without shell wrapping.
- Working directory: `jobs_dir` (recommended default).
- Environment: inherited minimal daemon environment; add explicit v1 vars:
  - `LICRON_JOB_NAME`
  - `LICRON_JOB_PATH`
  - `LICRON_SCHEDULE`

Concurrency:
- Global cap via `--max-parallel`.
- If due job’s prior process is still running: skip and log.
- Overlap skip is based on in-memory running state in v1 (single-instance model).

Timeout policy (hybrid):
- Global default timeout from `--default-timeout`.
- Optional per-job timeout from `# licron-timeout: <duration>`.
- Effective timeout selection:
  1. Use per-job timeout when present and valid.
  2. Otherwise use global default timeout.
  3. If selected timeout exceeds `--max-timeout`, cap it to `--max-timeout` and emit a warning log.

Exit handling:
- Record start time, end time, duration, exit status.
- Non-zero exit code is logged as failure; scheduler continues.
- On timeout, terminate process using staged signals:
  1. `SIGTERM`
  2. wait grace period (recommend 10s)
  3. `SIGKILL` if still running
- Timeout actions and final termination result must be logged.

Optional hardening allowed in v1 implementation:
- Configurable grace period between `SIGTERM` and `SIGKILL`.

---

## 10. Logging and Observability

Minimum required structured log events:
- `daemon_start`, `daemon_stop`
- `scan_started`, `scan_completed`
- `job_loaded`, `job_unloaded`, `job_updated`
- `job_ignored` (reason: not executable, invalid name, missing metadata, invalid cron)
- `job_triggered`
- `job_skipped_running`
- `job_exit` (exit code, duration)
- `job_exec_error` (spawn failure, permission issues)
- `job_timeout` (effective timeout reached)
- `job_kill_escalation` (`SIGTERM` sent, optional `SIGKILL` sent)

Log fields should include at least:
- timestamp
- job name/path (when applicable)
- reason/error message

---

## 11. Error Handling Policy

Startup should fail only for fatal daemon-level issues:
- invalid CLI usage
- unreadable/nonexistent `jobs_dir`
- inability to initialize scheduler core

Per-file/job errors are non-fatal:
- file is ignored
- reason is logged
- daemon remains running

---

## 12. Security Constraints

v1 minimum:
- Run as non-root dedicated user where possible.
- Execute only regular executable files within configured directory.
- Do not follow symlinks for execution target (recommended v1 default).
- Never evaluate metadata as code.

---

## 13. Determinism and Ordering

- Trigger eligibility is determined only by schedule and current time.
- No run-parts lexical ordering guarantee among simultaneously due jobs.
- Runtime ordering may vary based on scheduler wake timing and process spawning.

(This reflects product decision: **run purely on schedule time**.)

---

## 14. v1 Acceptance Criteria

A build is v1-complete when all are true:
- Daemon accepts a directory and continuously schedules valid jobs.
- Job discovery respects executable + naming constraints.
- Metadata parser correctly handles `# licron: <expr>` in first `N` lines.
- Metadata parser supports optional `# licron-timeout: <duration>` and enforces timeout policy.
- Invalid/missing metadata files are ignored and logged (no daemon crash).
- Overlapping triggers for same job are skipped and logged.
- Directory changes are detected and reflected in active schedule.
- Logs are sufficient to explain why each candidate was run, skipped, or ignored.
