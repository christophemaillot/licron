use std::collections::HashMap;
use std::path::Path;
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::model::{
    Cli, DEFAULT_TIMEOUT_GRACE_SECS, FULL_RESCAN_INTERVAL_SECS, IgnoredJob, JobRuntime, LoadedJob,
    RunningProc, RuntimeConfig, TimeMode,
};
use crate::platform::{
    date_parts_from_unix, format_system_time, send_sigterm, termination_requested, unix_secs,
};
use crate::scanner::scan_jobs;
use crate::watcher::create_watcher;

pub fn run_scheduler(
    loaded: Vec<LoadedJob>,
    max_parallel: usize,
    time_mode: TimeMode,
    jobs_dir: &Path,
    config: RuntimeConfig,
) {
    let mut states: Vec<JobRuntime> = loaded
        .into_iter()
        .map(|job| JobRuntime {
            job,
            running: None,
            schedule_enabled: true,
        })
        .collect();

    let mut last_seen_minute: Option<u64> = None;
    let mut watcher = create_watcher(jobs_dir);
    let mut last_full_rescan = Instant::now();

    eprintln!(
        "info: scheduler started jobs_dir={} jobs={} max_parallel={}",
        jobs_dir.display(),
        states.len(),
        max_parallel
    );

    loop {
        let now_inst = Instant::now();
        let now_sys = SystemTime::now();
        let mut active = 0usize;

        for state in &mut states {
            if let Some(running) = &mut state.running {
                match running.child.try_wait() {
                    Ok(Some(status)) => {
                        let elapsed = running.started_at.elapsed().as_secs();
                        eprintln!(
                            "event=job_exit job={} code={} duration_s={}",
                            state.job.path.display(),
                            status.code().map_or(-1, |c| c),
                            elapsed
                        );
                        state.running = None;
                        if !state.schedule_enabled {
                            eprintln!(
                                "event=job_unloaded_complete job={} reason=process_finished_after_unload",
                                state.job.path.display()
                            );
                        }
                    }
                    Ok(None) => {
                        active += 1;
                        if running.term_sent_at.is_none() && now_inst >= running.timeout_at {
                            let pid = running.child.id();
                            if send_sigterm(pid) {
                                eprintln!(
                                    "event=job_timeout job={} pid={} action=SIGTERM",
                                    state.job.path.display(),
                                    pid
                                );
                            } else {
                                eprintln!(
                                    "event=job_timeout job={} pid={} action=SIGTERM_FAILED",
                                    state.job.path.display(),
                                    pid
                                );
                            }
                            running.term_sent_at = Some(now_inst);
                        } else if let Some(term_at) = running.term_sent_at {
                            if now_inst.duration_since(term_at).as_secs()
                                >= DEFAULT_TIMEOUT_GRACE_SECS
                            {
                                let pid = running.child.id();
                                match running.child.kill() {
                                    Ok(_) => eprintln!(
                                        "event=job_kill_escalation job={} pid={} action=SIGKILL",
                                        state.job.path.display(),
                                        pid
                                    ),
                                    Err(err) => eprintln!(
                                        "event=job_kill_escalation job={} pid={} action=SIGKILL_FAILED err={}",
                                        state.job.path.display(),
                                        pid,
                                        err
                                    ),
                                }
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!(
                            "event=job_exec_error job={} reason=try_wait_failed err={}",
                            state.job.path.display(),
                            err
                        );
                    }
                }
            }
        }

        let minute_key = unix_secs(now_sys).map(|s| s as u64 / 60);
        if minute_key.is_some() && minute_key != last_seen_minute {
            last_seen_minute = minute_key;
            if let Some(unix) = unix_secs(now_sys) {
                if let Some(parts) = date_parts_from_unix(unix, time_mode) {
                    for state in &mut states {
                        if !state.schedule_enabled {
                            continue;
                        }
                        if !state.job.cron.matches(parts) {
                            continue;
                        }

                        if state.running.is_some() {
                            eprintln!(
                                "event=job_skipped_running job={} reason=already_running",
                                state.job.path.display()
                            );
                            continue;
                        }

                        if active >= max_parallel {
                            eprintln!(
                                "event=job_skipped_capacity job={} reason=max_parallel_reached max_parallel={}",
                                state.job.path.display(),
                                max_parallel
                            );
                            continue;
                        }

                        match spawn_job(&state.job.path, jobs_dir, &state.job.cron_expr) {
                            Ok(child) => {
                                let pid = child.id();
                                eprintln!(
                                    "event=job_triggered job={} pid={} timeout_s={}",
                                    state.job.path.display(),
                                    pid,
                                    state.job.timeout_secs
                                );
                                state.running = Some(RunningProc {
                                    child,
                                    started_at: now_inst,
                                    timeout_at: now_inst
                                        + Duration::from_secs(state.job.timeout_secs),
                                    term_sent_at: None,
                                });
                                active += 1;
                            }
                            Err(err) => {
                                eprintln!(
                                    "event=job_exec_error job={} reason=spawn_failed err={}",
                                    state.job.path.display(),
                                    err
                                );
                            }
                        }
                    }
                }
            }
        }

        states.retain(|state| state.schedule_enabled || state.running.is_some());

        match watcher.poll_changed(jobs_dir) {
            Ok(true) => {
                eprintln!("event=scan_started reason=watch_change_detected");
                reload_jobs(&mut states, jobs_dir, &config, time_mode);
                eprintln!(
                    "event=scan_completed jobs_loaded={}",
                    states.iter().filter(|s| s.schedule_enabled).count()
                );
            }
            Ok(false) => {}
            Err(err) => {
                eprintln!(
                    "event=scan_error jobs_dir={} err={}",
                    jobs_dir.display(),
                    err
                );
            }
        }

        if last_full_rescan.elapsed() >= Duration::from_secs(FULL_RESCAN_INTERVAL_SECS) {
            last_full_rescan = Instant::now();
            eprintln!(
                "event=scan_started reason=periodic_full_rescan interval_s={}",
                FULL_RESCAN_INTERVAL_SECS
            );
            reload_jobs(&mut states, jobs_dir, &config, time_mode);
            eprintln!(
                "event=scan_completed jobs_loaded={}",
                states.iter().filter(|s| s.schedule_enabled).count()
            );
        }

        if termination_requested() {
            eprintln!("info: shutdown signal received, draining running jobs");
            break;
        }

        thread::sleep(Duration::from_secs(1));
    }

    shutdown_running_jobs(&mut states);
    eprintln!("info: scheduler stopped");
}

fn reload_jobs(
    states: &mut Vec<JobRuntime>,
    jobs_dir: &Path,
    config: &RuntimeConfig,
    time_mode: TimeMode,
) {
    let (loaded, ignored) = scan_jobs(jobs_dir, config, time_mode);
    for item in &ignored {
        eprintln!(
            "event=job_ignored job={} reason={}",
            item.path.display(),
            item.reason
        );
    }

    let mut incoming: HashMap<std::path::PathBuf, LoadedJob> = loaded
        .into_iter()
        .map(|job| (job.path.clone(), job))
        .collect();
    let mut next_states = Vec::with_capacity(states.len() + incoming.len());

    for mut state in states.drain(..) {
        if let Some(new_job) = incoming.remove(&state.job.path) {
            if state.job.cron_expr != new_job.cron_expr
                || state.job.timeout_secs != new_job.timeout_secs
            {
                eprintln!(
                    "event=job_updated job={} cron='{}' timeout_s={}",
                    state.job.path.display(),
                    new_job.cron_expr,
                    new_job.timeout_secs
                );
            }
            state.job = new_job;
            state.schedule_enabled = true;
            next_states.push(state);
        } else if state.running.is_some() {
            if state.schedule_enabled {
                eprintln!(
                    "event=job_unloaded job={} reason=removed_or_invalid_will_wait_for_exit",
                    state.job.path.display()
                );
            }
            state.schedule_enabled = false;
            next_states.push(state);
        } else if state.schedule_enabled {
            eprintln!(
                "event=job_unloaded job={} reason=removed_or_invalid",
                state.job.path.display()
            );
        }
    }

    for (_, job) in incoming {
        eprintln!(
            "event=job_loaded job={} cron='{}' timeout_s={}",
            job.path.display(),
            job.cron_expr,
            job.timeout_secs
        );
        next_states.push(JobRuntime {
            job,
            running: None,
            schedule_enabled: true,
        });
    }

    next_states.sort_by(|a, b| a.job.path.cmp(&b.job.path));
    *states = next_states;
}

fn shutdown_running_jobs(states: &mut [JobRuntime]) {
    for state in states.iter_mut() {
        if let Some(running) = &mut state.running {
            let pid = running.child.id();
            if send_sigterm(pid) {
                eprintln!(
                    "event=job_shutdown_signal job={} pid={} action=SIGTERM",
                    state.job.path.display(),
                    pid
                );
            } else {
                eprintln!(
                    "event=job_shutdown_signal job={} pid={} action=SIGTERM_FAILED",
                    state.job.path.display(),
                    pid
                );
            }
            running.term_sent_at = Some(Instant::now());
        }
    }

    let deadline = Instant::now() + Duration::from_secs(DEFAULT_TIMEOUT_GRACE_SECS);
    loop {
        let mut remaining = 0usize;
        for state in states.iter_mut() {
            if let Some(running) = &mut state.running {
                match running.child.try_wait() {
                    Ok(Some(status)) => {
                        eprintln!(
                            "event=job_exit job={} code={} duration_s={}",
                            state.job.path.display(),
                            status.code().map_or(-1, |c| c),
                            running.started_at.elapsed().as_secs()
                        );
                        state.running = None;
                    }
                    Ok(None) => {
                        remaining += 1;
                    }
                    Err(err) => {
                        eprintln!(
                            "event=job_exec_error job={} reason=try_wait_failed_during_shutdown err={}",
                            state.job.path.display(),
                            err
                        );
                        remaining += 1;
                    }
                }
            }
        }
        if remaining == 0 {
            return;
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    for state in states.iter_mut() {
        if let Some(running) = &mut state.running {
            let pid = running.child.id();
            match running.child.kill() {
                Ok(_) => eprintln!(
                    "event=job_kill_escalation job={} pid={} action=SIGKILL shutdown=true",
                    state.job.path.display(),
                    pid
                ),
                Err(err) => eprintln!(
                    "event=job_kill_escalation job={} pid={} action=SIGKILL_FAILED shutdown=true err={}",
                    state.job.path.display(),
                    pid,
                    err
                ),
            }
            let _ = running.child.wait();
            state.running = None;
        }
    }
}

fn spawn_job(path: &Path, jobs_dir: &Path, cron_expr: &str) -> Result<Child, String> {
    Command::new(path)
        .current_dir(jobs_dir)
        .env("LICRON_JOB_NAME", file_name(path))
        .env("LICRON_JOB_PATH", path.display().to_string())
        .env("LICRON_SCHEDULE", cron_expr)
        .spawn()
        .map_err(|e| e.to_string())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

pub fn print_report(loaded: &[LoadedJob], ignored: &[IgnoredJob], cli: &Cli, mode: TimeMode) {
    println!("licron bootstrap report");
    println!("jobs_dir: {}", cli.jobs_dir.display());
    println!("timezone_mode: {:?}", mode);
    println!("max_parallel: {}", cli.max_parallel);
    println!("default_timeout: {}", cli.default_timeout);
    println!("max_timeout: {}", cli.max_timeout);
    println!("timeout_grace: {}s", DEFAULT_TIMEOUT_GRACE_SECS);
    println!();

    println!("loaded jobs: {}", loaded.len());
    for job in loaded {
        let next = job
            .next_run
            .and_then(|time| format_system_time(time, mode))
            .unwrap_or_else(|| "unavailable".to_string());
        println!(
            "- {} | cron='{}' | timeout={}s | next_run={}",
            job.path.display(),
            job.cron_expr,
            job.timeout_secs,
            next
        );
    }

    println!();
    println!("ignored jobs: {}", ignored.len());
    for job in ignored {
        println!("- {} | reason={}", job.path.display(), job.reason);
    }
}
