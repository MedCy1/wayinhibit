mod child;
mod cli;
mod pidfile;
mod signal;
mod wayland;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use child::ManagedChild;
use cli::{Config, ParseOutcome, parse_args};
use pidfile::Acquired;
use wayland::IdleInhibitor;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BIN_NAME: &str = env!("CARGO_PKG_NAME");
const CHILD_SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(2);

pub fn run_from_env() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match parse_args(&args) {
        Ok(ParseOutcome::Run(config)) => match run(config) {
            Ok(code) => code,
            Err(err) => {
                eprintln!("{BIN_NAME}: {err}");
                ExitCode::from(1)
            }
        },
        Ok(ParseOutcome::PrintHelp) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(ParseOutcome::PrintVersion) => {
            println!("{VERSION}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{BIN_NAME}: {err}");
            eprintln!("Run '{BIN_NAME} --help' for usage.");
            ExitCode::from(2)
        }
    }
}

fn print_help() {
    print!("{}", cli::help_text(VERSION));
}

fn run(config: Config) -> Result<ExitCode, String> {
    signal::install()?;

    // Take the pid file before anything else, so that "is an instance running?" and "become
    // the instance" are one atomic step, and an early failure below still cleans the file up.
    // The pid file is bookkeeping: if someone else owns it, the job (inhibiting idle, running
    // the command) still gets done, just without a pid file of our own. Only --toggle, whose
    // whole point is the owner, acts on it.
    let _pid_file = match config.pid_file.as_deref() {
        None => None,
        Some(path) => match pidfile::acquire(path)? {
            Acquired::Owner(pid_file) => {
                if let Some(reason) = pid_file.lock_problem() {
                    warn(&format!(
                        "cannot lock PID file '{}' ({reason}): a single instance is not \
                         guaranteed and --toggle may misjudge. Use a path on a local \
                         filesystem, e.g. under $XDG_RUNTIME_DIR, for reliable behavior.",
                        path.display()
                    ));
                }
                Some(pid_file)
            }
            Acquired::Held(Some(pid)) if config.toggle => return stop_instance(pid),
            Acquired::Held(None) if config.toggle => {
                return Err(format!(
                    "the instance owning PID file '{}' runs in another PID namespace and \
                     cannot be stopped from here",
                    path.display()
                ));
            }
            Acquired::Held(owner) => {
                let owner = owner.map_or_else(|| "another instance".into(), |p| format!("PID {p}"));
                warn(&format!(
                    "PID file '{}' is already in use by {owner}; running without it",
                    path.display()
                ));
                None
            }
        },
    };

    let mut inhibitor = if config.dry_run {
        None
    } else {
        Some(IdleInhibitor::connect(Duration::from_millis(250))?)
    };

    let deadline = config
        .timeout
        .map(|d| Instant::now().checked_add(d).ok_or("timeout is too large"))
        .transpose()?;

    if let Some(ref cmd) = config.on_inhibit {
        run_hook(cmd);
    }

    let prefix = if config.dry_run { "[dry-run] " } else { "" };

    let exit_code = match config.command {
        Some(command) => run_with_child(
            &mut inhibitor,
            command.spawn()?,
            config.quiet,
            deadline,
            prefix,
        )?,
        None => run_until_stopped(&mut inhibitor, config.quiet, deadline, prefix)?,
    };

    if let Some(ref mut inh) = inhibitor {
        inh.shutdown()?;
    }

    if let Some(ref cmd) = config.on_release {
        run_hook(cmd);
    }

    if !config.quiet {
        println!("\nStopped idle inhibition.");
    }

    Ok(exit_code)
}

/// Warnings go to stderr even with --quiet: they are about reliability, not progress.
fn warn(message: &str) {
    eprintln!("{BIN_NAME}: warning: {message}");
}

/// Asks the instance `pid` (the holder of the pid file lock) to stop.
fn stop_instance(pid: u32) -> Result<ExitCode, String> {
    match child::send_signal(pid, libc::SIGTERM) {
        Ok(()) => Ok(ExitCode::SUCCESS),
        // It exited on its own since we looked: the instance is stopped, as requested.
        Err(err) if err.raw_os_error() == Some(libc::ESRCH) => Ok(ExitCode::SUCCESS),
        Err(err) => Err(format!(
            "failed to stop running instance (PID {pid}): {err}"
        )),
    }
}

fn run_hook(cmd: &str) {
    let _ = std::process::Command::new("sh").arg("-c").arg(cmd).status();
}

/// Ticks the Wayland connection. If it has broken (compositor crash or restart), this
/// degrades to a plain sleep instead of failing the whole run: the caller (foreground loop
/// or wrapped command) keeps going, exit codes and hooks still fire normally, just without
/// idle inhibition for the remainder of the run.
fn tick(inhibitor: &mut Option<IdleInhibitor>) -> Result<(), String> {
    let Some(inh) = inhibitor.as_mut() else {
        std::thread::sleep(Duration::from_millis(250));
        return Ok(());
    };

    if let Some(notice) = inh.take_notice() {
        warn(&notice);
    }
    if let Err(err) = inh.tick() {
        eprintln!("{BIN_NAME}: idle inhibition lost ({err}); continuing without it.");
        *inhibitor = None;
    }
    Ok(())
}

fn run_until_stopped(
    inhibitor: &mut Option<IdleInhibitor>,
    quiet: bool,
    deadline: Option<Instant>,
    prefix: &str,
) -> Result<ExitCode, String> {
    if !quiet {
        println!(
            "{prefix}Inhibiting idle. PID: {}. Press Ctrl-C to stop.",
            std::process::id()
        );
    }

    while !signal::is_stop_requested() {
        if deadline.is_some_and(|dl| Instant::now() >= dl) {
            break;
        }
        tick(inhibitor)?;
    }

    Ok(ExitCode::SUCCESS)
}

fn run_with_child(
    inhibitor: &mut Option<IdleInhibitor>,
    mut child: ManagedChild,
    quiet: bool,
    deadline: Option<Instant>,
    prefix: &str,
) -> Result<ExitCode, String> {
    if !quiet {
        println!(
            "{prefix}Inhibiting idle while the child command is running. PID: {}. Press Ctrl-C to stop.",
            std::process::id()
        );
    }

    loop {
        if signal::is_stop_requested() || deadline.is_some_and(|dl| Instant::now() >= dl) {
            let code = child.terminate_and_wait(CHILD_SHUTDOWN_GRACE_PERIOD)?;
            return Ok(ExitCode::from(code));
        }

        if let Some(code) = child.try_wait()? {
            return Ok(ExitCode::from(code));
        }

        tick(inhibitor)?;
    }
}
