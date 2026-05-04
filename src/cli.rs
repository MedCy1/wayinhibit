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
  -q, --quiet      Suppress all output
  -h, --help       Print help
  -V, --version    Print version
"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub command: Option<CommandSpec>,
    pub quiet: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseOutcome {
    Run(Config),
    PrintHelp,
    PrintVersion,
}

pub fn parse_args(args: &[String]) -> Result<ParseOutcome, String> {
    if args.is_empty() {
        return Ok(ParseOutcome::Run(Config {
            command: None,
            quiet: false,
        }));
    }

    match args {
        [flag] if flag == "-h" || flag == "--help" => return Ok(ParseOutcome::PrintHelp),
        [flag] if flag == "-V" || flag == "--version" => return Ok(ParseOutcome::PrintVersion),
        _ => {}
    }

    let quiet = args.iter().any(|arg| arg == "-q" || arg == "--quiet");

    let remaining: Vec<&String> = args
        .iter()
        .filter(|arg| *arg != "-q" && *arg != "--quiet")
        .collect();

    if let Some(separator_index) = remaining.iter().position(|arg| *arg == "--") {
        let command_args = &remaining[separator_index + 1..];
        if command_args.is_empty() {
            return Err("expected a command after '--'".to_string());
        }

        return Ok(ParseOutcome::Run(Config {
            command: Some(CommandSpec {
                program: command_args[0].clone(),
                args: command_args[1..].iter().map(|s| (*s).clone()).collect(),
            }),
            quiet,
        }));
    }

    if remaining.is_empty() {
        return Ok(ParseOutcome::Run(Config {
            command: None,
            quiet,
        }));
    }

    Err(format!(
        "unsupported arguments: {}. Use '-- <COMMAND> [ARG...]' to run a command under inhibition.",
        remaining
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    ))
}

#[cfg(test)]
mod tests {
    use super::{Config, ParseOutcome, parse_args};
    use crate::child::CommandSpec;

    #[test]
    fn parses_foreground_mode() {
        let args: Vec<String> = Vec::new();

        assert_eq!(
            parse_args(&args),
            Ok(ParseOutcome::Run(Config {
                command: None,
                quiet: false,
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
            }))
        );
    }
}
