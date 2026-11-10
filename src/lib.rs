mod child;
mod cli;
mod pidfile;
mod signal;
mod wayland;

use std::ffi::OsString;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitCode};
use std::thread;
use std::time::{Duration, Instant};

use child::{ChildState, CommandSpec};
use cli::{Config, ParseOutcome, parse_args};
use pidfile::Acquired;
use wayland::IdleInhibitor;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BIN_NAME: &str = env!("CARGO_PKG_NAME");
const CHILD_SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(2);
/// How long a hook that is being stopped gets to end on SIGTERM before it is killed.
const HOOK_GRACE_PERIOD: Duration = Duration::from_secs(1);
/// How often a running hook is checked for completion and for a stop request.
const HOOK_POLL_INTERVAL: Duration = Duration::from_millis(20);
const WAYLAND_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// All output goes through these two functions. `println!` panics when the output has been
/// closed (`wayinhibit -- cmd | head`), which would replace the wrapped command's exit status
/// by 101. Blocking SIGTTOU keeps a background process (the terminal belongs to the wrapped
/// command) from being stopped by a terminal that restricts background writes.
fn out(text: &str) {
    write_quietly(std::io::stdout(), text);
}

fn err(text: &str) {
    write_quietly(std::io::stderr(), text);
}

fn write_quietly(mut stream: impl Write, text: &str) {
    child::with_sigttou_blocked(|| {
        let _ = stream.write_all(text.as_bytes());
        let _ = stream.flush();
    });
}

pub fn run_from_env() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();

    match parse_args(&args) {
        Ok(ParseOutcome::Run(config)) => match run(config) {
            Ok(code) => code,
            Err(message) => {
                err(&format!("{BIN_NAME}: {message}\n"));
                ExitCode::from(1)
            }
        },
        Ok(ParseOutcome::PrintHelp) => {
            out(&cli::help_text(VERSION));
            ExitCode::SUCCESS
        }
        Ok(ParseOutcome::PrintVersion) => {
            out(&format!("{VERSION}\n"));
            ExitCode::SUCCESS
        }
        Err(message) => {
            err(&format!(
                "{BIN_NAME}: {message}\nRun '{BIN_NAME} --help' for usage.\n"
            ));
            ExitCode::from(2)
        }
    }
}

/// The state of the idle inhibition for the whole run.
enum Inhibition {
    /// `--dry-run`: no compositor involved.
    DryRun,
    Active(Box<IdleInhibitor>),
    /// The compositor connection broke; nothing is inhibiting any more.
    Lost,
}

impl Inhibition {
    /// Waits for Wayland events for up to one poll interval (or just sleeps when there is no
    /// connection). Returns the reason when the connection has just broken.
    fn tick(&mut self) -> Option<String> {
        let Inhibition::Active(inhibitor) = self else {
            thread::sleep(WAYLAND_POLL_INTERVAL);
            return None;
        };

        if let Some(notice) = inhibitor.take_notice() {
            warn(&notice);
        }
        match inhibitor.tick() {
            Ok(()) => None,
            Err(reason) => {
                *self = Inhibition::Lost;
                Some(reason)
            }
        }
    }
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

    let mut inhibition = if config.dry_run {
        Inhibition::DryRun
    } else {
        Inhibition::Active(Box::new(IdleInhibitor::connect(WAYLAND_POLL_INTERVAL)?))
    };

    let deadline = config
        .timeout
        .map(|d| Instant::now().checked_add(d).ok_or("timeout is too large"))
        .transpose()?;

    // A stop that arrived while connecting means there is nothing left to do, and no hook may
    // run: the release hook only ever follows a start hook.
    let started = !signal::is_stop_requested();
    if started {
        if let Some(cmd) = &config.on_inhibit {
            run_hook(cmd);
        }
    }

    let prefix = if config.dry_run { "[dry-run] " } else { "" };

    // From here on every path, errors included, must go through the cleanup below.
    let outcome = if signal::is_stop_requested() {
        Ok(ExitCode::SUCCESS)
    } else {
        match &config.command {
            Some(command) => {
                run_with_command(&mut inhibition, command, config.quiet, deadline, prefix)
            }
            None => run_until_stopped(&mut inhibition, config.quiet, deadline, prefix),
        }
    };

    if let Inhibition::Active(inhibitor) = &mut inhibition {
        if let Err(reason) = inhibitor.shutdown() {
            warn(&format!("failed to release the idle inhibition: {reason}"));
        }
    }

    if started {
        if let Some(cmd) = &config.on_release {
            run_hook(cmd);
        }
    }

    if !config.quiet {
        out("\nStopped idle inhibition.\n");
    }

    outcome
}

/// Warnings go to stderr even with --quiet: they are about reliability, not progress.
fn warn(message: &str) {
    err(&format!("{BIN_NAME}: warning: {message}\n"));
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

/// Runs a user hook through `sh -c`. It runs in its own process group and is watched, so that
/// a stop request is not held up by a slow hook: the request that arrives while it runs (the
/// first one for a hook that runs before the stop, the second for the release hook, which
/// runs because of the first) ends it. Its exit status is ignored.
fn run_hook(cmd: &str) {
    let abort_at = signal::stop_requests().saturating_add(1);

    let Ok(mut hook) = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .process_group(0)
        .spawn()
    else {
        return;
    };

    loop {
        match hook.try_wait() {
            Ok(None) => {}
            Ok(Some(_)) | Err(_) => return,
        }
        if signal::stop_requests() >= abort_at {
            end_hook(&mut hook);
            return;
        }
        thread::sleep(HOOK_POLL_INTERVAL);
    }
}

/// Asks a hook (and what it started) to terminate, then kills it if it does not.
fn end_hook(hook: &mut Child) {
    let _ = child::send_signal_to_group(hook.id(), libc::SIGTERM);

    let deadline = Instant::now() + HOOK_GRACE_PERIOD;
    while Instant::now() < deadline {
        if matches!(hook.try_wait(), Ok(Some(_)) | Err(_)) {
            return;
        }
        thread::sleep(HOOK_POLL_INTERVAL);
    }

    let _ = child::send_signal_to_group(hook.id(), libc::SIGKILL);
    let _ = hook.wait();
}

fn run_until_stopped(
    inhibition: &mut Inhibition,
    quiet: bool,
    deadline: Option<Instant>,
    prefix: &str,
) -> Result<ExitCode, String> {
    if !quiet {
        out(&format!(
            "{prefix}Inhibiting idle. PID: {}. Press Ctrl-C to stop.\n",
            std::process::id()
        ));
    }

    while !signal::is_stop_requested() {
        if deadline.is_some_and(|dl| Instant::now() >= dl) {
            break;
        }
        // With nothing to run, a broken connection means nothing is inhibiting any more:
        // say so and fail instead of waiting forever with a pid file that claims otherwise.
        if let Some(reason) = inhibition.tick() {
            return Err(format!("idle inhibition lost ({reason})"));
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn run_with_command(
    inhibition: &mut Inhibition,
    command: &CommandSpec,
    quiet: bool,
    deadline: Option<Instant>,
    prefix: &str,
) -> Result<ExitCode, String> {
    let mut child = command.spawn()?;

    if !quiet {
        out(&format!(
            "{prefix}Inhibiting idle while the child command is running. PID: {}. Press Ctrl-C to stop.\n",
            std::process::id()
        ));
    }

    loop {
        if signal::is_stop_requested() || deadline.is_some_and(|dl| Instant::now() >= dl) {
            let code = child.terminate_and_wait(CHILD_SHUTDOWN_GRACE_PERIOD)?;
            return Ok(ExitCode::from(code));
        }

        match child.poll()? {
            ChildState::Exited(code) => return Ok(ExitCode::from(code)),
            ChildState::Stopped => child.relay_stop(),
            ChildState::Running => {}
        }

        // The command is what matters here: it keeps running, just without inhibition.
        if let Some(reason) = inhibition.tick() {
            warn(&format!(
                "idle inhibition lost ({reason}); the command keeps running without it."
            ));
        }
    }
}
