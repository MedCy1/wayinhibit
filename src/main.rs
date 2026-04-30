use std::env;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BIN_NAME: &str = env!("CARGO_PKG_NAME");

fn print_help() {
    println!(
        "\
{BIN_NAME} {VERSION}

A small Wayland idle inhibitor written in Rust.

Usage:
  {BIN_NAME} [OPTIONS]

Options:
  -h, --help       Print help
  -V, --version    Print version
"
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        eprintln!("{BIN_NAME}: implementation not available yet.");
        eprintln!("Run '{BIN_NAME} --help' for the current interface.");
        return ExitCode::from(1);
    }

    match args.as_slice() {
        [flag] if flag == "-h" || flag == "--help" => {
            print_help();
            ExitCode::SUCCESS
        }
        [flag] if flag == "-V" || flag == "--version" => {
            println!("{VERSION}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{BIN_NAME}: unsupported arguments: {}", args.join(" "));
            eprintln!("Run '{BIN_NAME} --help' for usage.");
            ExitCode::from(2)
        }
    }
}
