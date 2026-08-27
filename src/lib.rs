mod child;
mod cli;
mod signal;
mod wayland;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use child::ManagedChild;
use cli::{Config, ParseOutcome, parse_args};
use wayland::IdleInhibitor;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BIN_NAME: &str = env!("CARGO_PKG_NAME");
const CHILD_SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(2);

struct PidFile(PathBuf);

impl PidFile {
    fn create(path: &Path) -> Result<Self, String> {
        let pid = std::process::id();
        std::fs::write(path, format!("{pid}\n"))
            .map_err(|err| format!("failed to write PID file '{}': {err}", path.display()))?;
        Ok(Self(path.to_owned()))
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

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

/// Reads a PID file and returns the PID if the process it names is still alive.
fn running_pid(path: &Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(path).ok()?.trim().parse().ok()?;
    let signed_pid = i32::try_from(pid).ok()?;
    // Signal 0 sends nothing; it only checks whether the process exists.
    (unsafe { libc::kill(signed_pid, 0) } == 0).then_some(pid)
}

fn run(config: Config) -> Result<ExitCode, String> {
    if config.toggle {
        let pid_path = config
            .pid_file
            .as_deref()
            .expect("cli::parse_args guarantees --toggle implies --pid-file");
        if let Some(pid) = running_pid(pid_path) {
            child::send_signal(pid, libc::SIGTERM)
                .map_err(|err| format!("failed to stop running instance (PID {pid}): {err}"))?;
            return Ok(ExitCode::SUCCESS);
        }
        // No running instance: fall through and start one normally.
    }

    signal::install()?;

    let mut inhibitor = if config.dry_run {
        None
    } else {
        Some(IdleInhibitor::connect(Duration::from_millis(250))?)
    };

    let deadline = config.timeout.map(|d| Instant::now() + d);

    let _pid_file = config
        .pid_file
        .as_deref()
        .map(PidFile::create)
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

    if let Err(err) = inh.tick() {
        eprintln!(
            "{BIN_NAME}: lost the Wayland connection ({err}); continuing without idle inhibition."
        );
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
