mod child;
mod cli;
mod signal;
mod wayland;

use std::process::ExitCode;
use std::time::Duration;

use child::ManagedChild;
use cli::{Config, ParseOutcome, parse_args};
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
    signal::reset();
    signal::install()?;

    let mut inhibitor = IdleInhibitor::connect(Duration::from_millis(250))?;

    let exit_code = match config.command {
        Some(command) => run_with_child(&mut inhibitor, command.spawn()?, config.quiet)?,
        None => run_until_stopped(&mut inhibitor, config.quiet)?,
    };

    inhibitor.shutdown()?;

    if !config.quiet {
        println!("\nStopped idle inhibition.");
    }

    Ok(exit_code)
}

fn run_until_stopped(inhibitor: &mut IdleInhibitor, quiet: bool) -> Result<ExitCode, String> {
    if !quiet {
        println!(
            "Inhibiting idle. PID: {}. Press Ctrl-C to stop.",
            std::process::id()
        );
    }

    while !signal::is_stop_requested() {
        inhibitor.tick()?;
    }

    Ok(ExitCode::SUCCESS)
}

fn run_with_child(
    inhibitor: &mut IdleInhibitor,
    mut child: ManagedChild,
    quiet: bool,
) -> Result<ExitCode, String> {
    if !quiet {
        println!(
            "Inhibiting idle while the child command is running. PID: {}. Press Ctrl-C to stop.",
            std::process::id()
        );
    }

    loop {
        if signal::is_stop_requested() {
            let code = child.terminate_and_wait(CHILD_SHUTDOWN_GRACE_PERIOD)?;
            return Ok(ExitCode::from(code));
        }

        if let Some(code) = child.try_wait()? {
            return Ok(ExitCode::from(code));
        }

        inhibitor.tick()?;
    }
}
