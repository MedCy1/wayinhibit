use std::path::PathBuf;
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
  -t, --timeout <DURATION>      Stop after a given duration (e.g. 30s, 5m, 2h)
  -q, --quiet                   Suppress all output
  -p, --pid-file <PATH>         Write PID to PATH on startup, remove it on exit
      --on-inhibit <CMD>        Run CMD (via sh -c) when inhibition starts
      --on-release <CMD>        Run CMD (via sh -c) when inhibition stops
  -h, --help                    Print help
  -V, --version                 Print version
"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub command: Option<CommandSpec>,
    pub quiet: bool,
    pub timeout: Option<Duration>,
    pub pid_file: Option<PathBuf>,
    pub on_inhibit: Option<String>,
    pub on_release: Option<String>,
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
            pid_file: None,
            on_inhibit: None,
            on_release: None,
        }));
    }

    match args {
        [flag] if flag == "-h" || flag == "--help" => return Ok(ParseOutcome::PrintHelp),
        [flag] if flag == "-V" || flag == "--version" => return Ok(ParseOutcome::PrintVersion),
        _ => {}
    }

    let mut quiet = false;
    let mut timeout = None;
    let mut pid_file = None;
    let mut on_inhibit = None;
    let mut on_release = None;
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
            flag if flag.starts_with("--timeout=") => {
                let val = flag.strip_prefix("--timeout=").unwrap();
                timeout = Some(parse_duration(val)?);
            }
            "-p" | "--pid-file" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--pid-file requires a value".to_string())?;
                pid_file = Some(PathBuf::from(val));
            }
            flag if flag.starts_with("--pid-file=") => {
                let val = flag.strip_prefix("--pid-file=").unwrap();
                if val.is_empty() {
                    return Err("--pid-file requires a value".to_string());
                }
                pid_file = Some(PathBuf::from(val));
            }
            "--on-inhibit" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--on-inhibit requires a value".to_string())?;
                on_inhibit = Some(val.clone());
            }
            flag if flag.starts_with("--on-inhibit=") => {
                let val = flag.strip_prefix("--on-inhibit=").unwrap();
                if val.is_empty() {
                    return Err("--on-inhibit requires a value".to_string());
                }
                on_inhibit = Some(val.to_string());
            }
            "--on-release" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--on-release requires a value".to_string())?;
                on_release = Some(val.clone());
            }
            flag if flag.starts_with("--on-release=") => {
                let val = flag.strip_prefix("--on-release=").unwrap();
                if val.is_empty() {
                    return Err("--on-release requires a value".to_string());
                }
                on_release = Some(val.to_string());
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
        pid_file,
        on_inhibit,
        on_release,
    }))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use std::path::PathBuf;

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
                pid_file: None,
                on_inhibit: None,
                on_release: None,
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
                pid_file: None,
                on_inhibit: None,
                on_release: None,
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
                pid_file: None,
                on_inhibit: None,
                on_release: None,
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
                pid_file: None,
                on_inhibit: None,
                on_release: None,
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
                pid_file: None,
                on_inhibit: None,
                on_release: None,
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
                pid_file: None,
                on_inhibit: None,
                on_release: None,
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
                pid_file: None,
                on_inhibit: None,
                on_release: None,
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
                pid_file: None,
                on_inhibit: None,
                on_release: None,
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
                pid_file: None,
                on_inhibit: None,
                on_release: None,
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
                pid_file: None,
                on_inhibit: None,
                on_release: None,
            }))
        );
    }

    #[test]
    fn parses_timeout_inline_equals() {
        let args = vec!["--timeout=30s".to_string()];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: false,
                timeout: Some(Duration::from_secs(30)),
                pid_file: None,
                on_inhibit: None,
                on_release: None,
            }))
        );
    }

    #[test]
    fn parses_timeout_inline_equals_minutes() {
        let args = vec!["--timeout=5m".to_string()];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: false,
                timeout: Some(Duration::from_secs(300)),
                pid_file: None,
                on_inhibit: None,
                on_release: None,
            }))
        );
    }

    #[test]
    fn rejects_timeout_inline_equals_invalid_duration() {
        let args = vec!["--timeout=abc".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn rejects_timeout_inline_equals_empty_value() {
        let args = vec!["--timeout=".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn parses_pid_file() {
        let args = vec!["--pid-file".to_string(), "/run/wayinhibit.pid".to_string()];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: false,
                timeout: None,
                pid_file: Some(PathBuf::from("/run/wayinhibit.pid")),
                on_inhibit: None,
                on_release: None,
            }))
        );
    }

    #[test]
    fn parses_pid_file_short_flag() {
        let args = vec!["-p".to_string(), "/tmp/wi.pid".to_string()];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: false,
                timeout: None,
                pid_file: Some(PathBuf::from("/tmp/wi.pid")),
                on_inhibit: None,
                on_release: None,
            }))
        );
    }

    #[test]
    fn parses_pid_file_inline_equals() {
        let args = vec!["--pid-file=/tmp/wi.pid".to_string()];
        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: false,
                timeout: None,
                pid_file: Some(PathBuf::from("/tmp/wi.pid")),
                on_inhibit: None,
                on_release: None,
            }))
        );
    }

    #[test]
    fn rejects_pid_file_without_value() {
        let args = vec!["--pid-file".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn rejects_pid_file_inline_equals_empty_value() {
        let args = vec!["--pid-file=".to_string()];
        assert!(parse_args(&args).is_err());
    }
}
