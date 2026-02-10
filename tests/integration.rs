use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);
static TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    match TEST_MUTEX.get_or_init(|| Mutex::new(())).lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn bin_path() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_licron") {
        return PathBuf::from(path);
    }

    let exe = std::env::current_exe().expect("resolve current test executable path");
    let debug_dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("resolve target debug directory");
    let mut candidate = debug_dir.join("licron");
    if cfg!(target_os = "windows") {
        candidate.set_extension("exe");
    }
    assert!(
        candidate.exists(),
        "could not find licron binary at {} and CARGO_BIN_EXE_licron is not set",
        candidate.display()
    );
    candidate
}

fn unique_temp_dir(prefix: &str) -> io::Result<PathBuf> {
    let mut path = std::env::temp_dir();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let seq = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.push(format!("licron-{prefix}-{}-{seq}", now));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn write_executable(path: &Path, content: &str) -> io::Result<()> {
    fs::write(path, content)?;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> io::Result<()> {
    let start = SystemTime::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        let elapsed = SystemTime::now()
            .duration_since(start)
            .unwrap_or_else(|_| Duration::from_secs(0));
        if elapsed >= timeout {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "process did not exit within timeout",
    ))
}

fn send_sigterm(pid: u32) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
}

fn out_to_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

#[test]
fn oneshot_examples_report_expected_counts() {
    let _guard = test_guard();
    let bin = bin_path();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let valid = Command::new(&bin)
        .arg("--oneshot")
        .arg(root.join("examples/jobs"))
        .output()
        .expect("run oneshot against valid examples");
    assert!(valid.status.success(), "valid examples command failed");
    let valid_out = out_to_string(&valid.stdout);
    assert!(
        valid_out.contains("loaded jobs: 5"),
        "expected loaded jobs count in output:\n{valid_out}"
    );
    assert!(
        valid_out.contains("ignored jobs: 0"),
        "expected ignored jobs count in output:\n{valid_out}"
    );

    let invalid = Command::new(&bin)
        .arg("--oneshot")
        .arg(root.join("examples/invalid-jobs"))
        .output()
        .expect("run oneshot against invalid examples");
    assert!(
        invalid.status.success(),
        "invalid examples command should still succeed"
    );
    let invalid_out = out_to_string(&invalid.stdout);
    assert!(
        invalid_out.contains("loaded jobs: 0"),
        "expected loaded jobs count in output:\n{invalid_out}"
    );
    assert!(
        invalid_out.contains("ignored jobs: 3"),
        "expected ignored jobs count in output:\n{invalid_out}"
    );
}

#[test]
fn timezone_iana_value_is_accepted() {
    let _guard = test_guard();
    let bin = bin_path();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let out = Command::new(&bin)
        .arg("--oneshot")
        .arg("--timezone")
        .arg("Etc/UTC")
        .arg(root.join("examples/jobs"))
        .output()
        .expect("run oneshot with IANA timezone");

    assert!(out.status.success(), "timezone run should succeed");
    let stdout = out_to_string(&out.stdout);
    assert!(
        stdout.contains("loaded jobs: 5"),
        "expected normal scan output, got:\n{stdout}"
    );
}

#[test]
fn timezone_invalid_value_is_rejected() {
    let _guard = test_guard();
    let bin = bin_path();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let out = Command::new(&bin)
        .arg("--oneshot")
        .arg("--timezone")
        .arg("Mars/Phobos")
        .arg(root.join("examples/jobs"))
        .output()
        .expect("run oneshot with invalid timezone");

    assert_eq!(out.status.code(), Some(2), "invalid timezone should fail");
    let stderr = out_to_string(&out.stderr);
    assert!(
        stderr.contains("invalid timezone"),
        "expected invalid timezone error, got:\n{stderr}"
    );
}

#[test]
fn daemon_enforces_single_instance_lock() {
    let _guard = test_guard();
    let bin = bin_path();
    let dir = unique_temp_dir("lock").expect("create temp dir");

    write_executable(
        &dir.join("01-lock.sh"),
        "#!/usr/bin/env bash\n# licron: * * * * *\n# licron-timeout: 20s\nsleep 2\n",
    )
    .expect("write job file");

    let mut first = Command::new(&bin)
        .arg(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn first daemon");

    thread::sleep(Duration::from_millis(600));

    let second: Output = Command::new(&bin)
        .arg(&dir)
        .output()
        .expect("run second daemon");
    assert_eq!(
        second.status.code(),
        Some(2),
        "second instance should fail with exit code 2"
    );
    let second_err = out_to_string(&second.stderr);
    assert!(
        second_err.contains("another licron instance appears active"),
        "expected lock contention error, got:\n{second_err}"
    );

    send_sigterm(first.id());
    wait_for_exit(&mut first, Duration::from_secs(5)).expect("first daemon should exit after TERM");

    let mut third = Command::new(&bin)
        .arg(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn third daemon after lock release");

    thread::sleep(Duration::from_millis(600));
    send_sigterm(third.id());
    wait_for_exit(&mut third, Duration::from_secs(5))
        .expect("third daemon should run and stop cleanly");
}

#[test]
fn daemon_starts_and_shuts_down_gracefully() {
    let _guard = test_guard();
    let bin = bin_path();
    let dir = unique_temp_dir("shutdown").expect("create temp dir");

    let first = dir.join("01-a.sh");
    write_executable(
        &first,
        "#!/usr/bin/env bash\n# licron: * * * * *\n# licron-timeout: 20s\nsleep 2\n",
    )
    .expect("write initial job");

    let stderr_path = dir.join("daemon.stderr.log");
    let stderr_file = fs::File::create(&stderr_path).expect("create stderr log file");

    let mut daemon = Command::new(&bin)
        .arg(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .expect("spawn daemon");

    thread::sleep(Duration::from_secs(2));

    send_sigterm(daemon.id());
    wait_for_exit(&mut daemon, Duration::from_secs(8)).expect("daemon should exit after TERM");

    let stderr_bytes = fs::read(&stderr_path).expect("read daemon stderr log");
    let stderr = out_to_string(&stderr_bytes);
    assert!(
        stderr.contains("scheduler started"),
        "expected scheduler startup in stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("shutdown signal received"),
        "expected shutdown signal handling in stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("scheduler stopped"),
        "expected scheduler stop log in stderr:\n{stderr}"
    );
    assert!(
        stderr.contains(".licron.lock"),
        "expected instance lock logs in stderr:\n{stderr}"
    );
}

#[test]
fn daemon_times_out_and_escalates_kill() {
    let _guard = test_guard();
    let bin = bin_path();
    let dir = unique_temp_dir("timeout").expect("create temp dir");

    let job = dir.join("01-timeout.sh");
    write_executable(
        &job,
        "#!/usr/bin/env bash\n# licron: * * * * *\n# licron-timeout: 3s\ntrap '' TERM\nwhile true; do sleep 1; done\n",
    )
    .expect("write timeout job");

    let stderr_path = dir.join("daemon.stderr.log");
    let stderr_file = fs::File::create(&stderr_path).expect("create stderr log file");

    let mut daemon = Command::new(&bin)
        .arg(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .expect("spawn daemon");

    thread::sleep(Duration::from_secs(20));

    send_sigterm(daemon.id());
    wait_for_exit(&mut daemon, Duration::from_secs(8)).expect("daemon should exit after TERM");

    let stderr = out_to_string(&fs::read(&stderr_path).expect("read daemon stderr log"));
    let timeout_expected = format!("event=job_timeout job={}", job.display());
    let kill_expected = format!("event=job_kill_escalation job={}", job.display());

    assert!(
        stderr.contains(&timeout_expected),
        "expected timeout event in stderr:\n{stderr}"
    );
    assert!(
        stderr.contains(&kill_expected) && stderr.contains("action=SIGKILL"),
        "expected SIGKILL escalation in stderr:\n{stderr}"
    );
}

#[test]
fn daemon_skips_overlapping_runs() {
    let _guard = test_guard();
    let bin = bin_path();
    let dir = unique_temp_dir("overlap").expect("create temp dir");

    let job = dir.join("01-overlap.sh");
    write_executable(
        &job,
        "#!/usr/bin/env bash\n# licron: * * * * *\n# licron-timeout: 180s\nsleep 75\n",
    )
    .expect("write overlap job");

    let stderr_path = dir.join("daemon.stderr.log");
    let stderr_file = fs::File::create(&stderr_path).expect("create stderr log file");

    let mut daemon = Command::new(&bin)
        .arg(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .expect("spawn daemon");

    thread::sleep(Duration::from_secs(72));

    send_sigterm(daemon.id());
    wait_for_exit(&mut daemon, Duration::from_secs(12)).expect("daemon should exit after TERM");

    let stderr = out_to_string(&fs::read(&stderr_path).expect("read daemon stderr log"));
    let skip_expected = format!("event=job_skipped_running job={}", job.display());

    assert!(
        stderr.contains(&skip_expected),
        "expected overlap skip event in stderr:\n{stderr}"
    );
}
