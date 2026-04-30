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
    println!("{}", cli::HELP_TEXT.replace("0.1.0", VERSION));
}

fn run(config: Config) -> Result<ExitCode, String> {
    signal::reset();
    signal::install()?;

    let mut inhibitor = IdleInhibitor::connect(Duration::from_millis(250))?;

    let exit_code = match config.command {
        Some(command) => run_with_child(&mut inhibitor, command.spawn()?)?,
        None => run_until_stopped(&mut inhibitor)?,
    };

    inhibitor.shutdown()?;

    println!("\nStopped idle inhibition.");

    Ok(exit_code)
}

fn run_until_stopped(inhibitor: &mut IdleInhibitor) -> Result<ExitCode, String> {
    println!("Inhibiting idle. Press Ctrl-C to stop.");

    while !signal::is_stop_requested() {
        inhibitor.tick()?;
    }

    Ok(ExitCode::SUCCESS)
}

fn run_with_child(
    inhibitor: &mut IdleInhibitor,
    mut child: ManagedChild,
) -> Result<ExitCode, String> {
    println!("Inhibiting idle while the child command is running. Press Ctrl-C to stop.");

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
