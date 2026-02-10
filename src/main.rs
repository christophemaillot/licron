mod cli;
mod cron;
mod model;
mod platform;
mod scanner;
mod scheduler;
mod watcher;

use cli::{build_runtime_config, parse_cli, print_usage, validate_jobs_dir};
use platform::{acquire_instance_lock, install_signal_handlers, resolve_time_mode};
use scanner::scan_jobs;
use scheduler::{print_report, run_scheduler};

fn main() {
    let cli = match parse_cli(std::env::args().collect()) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("error: {err}");
            print_usage();
            std::process::exit(2);
        }
    };

    if let Err(err) = validate_jobs_dir(&cli.jobs_dir) {
        eprintln!("error: {err}");
        std::process::exit(2);
    }

    let config = match build_runtime_config(&cli) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(2);
        }
    };

    let time_mode = match resolve_time_mode(&cli.timezone) {
        Ok(mode) => mode,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(2);
        }
    };

    let (loaded, ignored) = scan_jobs(&cli.jobs_dir, &config, time_mode);
    print_report(&loaded, &ignored, &cli, time_mode);

    if cli.dry_run || cli.oneshot {
        return;
    }

    let _instance_lock = match acquire_instance_lock(&cli.jobs_dir) {
        Ok(lock) => lock,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(2);
        }
    };

    if let Err(err) = install_signal_handlers() {
        eprintln!("warn: failed to install signal handlers: {err}");
    }

    run_scheduler(loaded, cli.max_parallel, time_mode, &cli.jobs_dir, config);
}
