use std::time::Duration;

use crate::child::CommandSpec;

pub fn help_text(version: &str) -> String {
    format!(
        "\
wayinhibit {version}

A small Wayland idle inhibitor written in Rust.

Usage:
  wayinhibit [OPTIONS]
  wayinhibit [OPTIONS] -- <COMMAND> [ARG...]

Options:
  -t, --timeout <DURATION>  Stop after a given duration (e.g. 30s, 5m, 2h)
  -q, --quiet               Suppress all output
  -h, --help                Print help
  -V, --version             Print version
"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub command: Option<CommandSpec>,
    pub quiet: bool,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseOutcome {
    Run(Config),
    PrintHelp,
    PrintVersion,
}

pub fn parse_duration(s: &str) -> Result<Duration, String> {
    if let Some(h) = s.strip_suffix('h') {
        let hours: u64 = h
            .parse()
            .map_err(|_| format!("invalid duration '{s}': expected e.g. 30s, 5m, 2h"))?;
        return Ok(Duration::from_secs(hours * 3600));
    }
    if let Some(m) = s.strip_suffix('m') {
        let minutes: u64 = m
            .parse()
            .map_err(|_| format!("invalid duration '{s}': expected e.g. 30s, 5m, 2h"))?;
        return Ok(Duration::from_secs(minutes * 60));
    }
    if let Some(s_) = s.strip_suffix('s') {
        let secs: u64 = s_
            .parse()
            .map_err(|_| format!("invalid duration '{s}': expected e.g. 30s, 5m, 2h"))?;
        return Ok(Duration::from_secs(secs));
    }
    Err(format!(
        "invalid duration '{s}': expected a value with a unit suffix, e.g. 30s, 5m, 2h"
    ))
}

pub fn parse_args(args: &[String]) -> Result<ParseOutcome, String> {
    if args.is_empty() {
        return Ok(ParseOutcome::Run(Config {
            command: None,
            quiet: false,
            timeout: None,
        }));
    }

    match args {
        [flag] if flag == "-h" || flag == "--help" => return Ok(ParseOutcome::PrintHelp),
        [flag] if flag == "-V" || flag == "--version" => return Ok(ParseOutcome::PrintVersion),
        _ => {}
    }

    let mut quiet = false;
    let mut timeout = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-q" | "--quiet" => quiet = true,
            "-t" | "--timeout" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--timeout requires a value".to_string())?;
                timeout = Some(parse_duration(val)?);
            }
            "--" => break,
            other => {
                return Err(format!(
                    "unsupported argument: {other}. Run '{} --help' for usage.",
                    env!("CARGO_PKG_NAME")
                ));
            }
        }
        i += 1;
    }

    let command = if i < args.len() && args[i] == "--" {
        let command_args = &args[i + 1..];
        if command_args.is_empty() {
            return Err("expected a command after '--'".to_string());
        }
        Some(CommandSpec {
            program: command_args[0].clone(),
            args: command_args[1..].to_vec(),
        })
    } else {
        None
    };

    Ok(ParseOutcome::Run(Config {
        command,
        quiet,
        timeout,
    }))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Config, ParseOutcome, parse_args, parse_duration};
    use crate::child::CommandSpec;

    #[test]
    fn parses_foreground_mode() {
        assert_eq!(
            parse_args(&[]),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: false,
                timeout: None,
            }))
        );
    }

    #[test]
    fn parses_help() {
        let args = vec!["--help".to_string()];
        assert_eq!(parse_args(&args), Ok(ParseOutcome::PrintHelp));
    }

    #[test]
    fn parses_version() {
        let args = vec!["--version".to_string()];
        assert_eq!(parse_args(&args), Ok(ParseOutcome::PrintVersion));
    }

    #[test]
    fn parses_command_after_separator() {
        let args = vec!["--".to_string(), "sleep".to_string(), "1".to_string()];

        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: Some(CommandSpec {
                    program: "sleep".to_string(),
                    args: vec!["1".to_string()],
                }),
                quiet: false,
                timeout: None,
            }))
        );
    }

    #[test]
    fn rejects_empty_command_after_separator() {
        let args = vec!["--".to_string()];
        assert_eq!(
            parse_args(&args),
            Err("expected a command after '--'".to_string())
        );
    }

    #[test]
    fn rejects_bare_arguments() {
        let args = vec!["sleep".to_string(), "1".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn parses_quiet_flag() {
        let args = vec!["--quiet".to_string()];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: true,
                timeout: None,
            }))
        );
    }

    #[test]
    fn parses_quiet_short_flag() {
        let args = vec!["-q".to_string()];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: true,
                timeout: None,
            }))
        );
    }

    #[test]
    fn parses_quiet_with_command() {
        let args = vec![
            "-q".to_string(),
            "--".to_string(),
            "sleep".to_string(),
            "1".to_string(),
        ];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: Some(CommandSpec {
                    program: "sleep".to_string(),
                    args: vec!["1".to_string()],
                }),
                quiet: true,
                timeout: None,
            }))
        );
    }

    #[test]
    fn parses_timeout_seconds() {
        let args = vec!["--timeout".to_string(), "30s".to_string()];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: false,
                timeout: Some(Duration::from_secs(30)),
            }))
        );
    }

    #[test]
    fn parses_timeout_minutes() {
        let args = vec!["-t".to_string(), "5m".to_string()];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: false,
                timeout: Some(Duration::from_secs(300)),
            }))
        );
    }

    #[test]
    fn parses_timeout_hours() {
        let args = vec!["-t".to_string(), "2h".to_string()];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: false,
                timeout: Some(Duration::from_secs(7200)),
            }))
        );
    }

    #[test]
    fn parses_timeout_with_command() {
        let args = vec![
            "-t".to_string(),
            "10m".to_string(),
            "--".to_string(),
            "rsync".to_string(),
        ];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: Some(CommandSpec {
                    program: "rsync".to_string(),
                    args: vec![],
                }),
                quiet: false,
                timeout: Some(Duration::from_secs(600)),
            }))
        );
    }

    #[test]
    fn rejects_timeout_without_value() {
        let args = vec!["--timeout".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn rejects_invalid_duration() {
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("5").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn parses_quiet_and_timeout_combined() {
        let args = vec!["-q".to_string(), "-t".to_string(), "5m".to_string()];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: true,
                timeout: Some(Duration::from_secs(300)),
            }))
        );
    }
}
