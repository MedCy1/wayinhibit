//! The README, the manual page, `--help` and the shell completions describe the same tool.
//!
//! They are written by hand in six places, and they drifted apart before. This checks what a
//! machine can check: every option appears everywhere, and the statements that were once wrong
//! stay in all the places that must carry them.

use std::path::PathBuf;
use std::process::Command;

fn read(relative: &str) -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), relative].iter().collect();
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn help() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_wayinhibit"))
        .arg("--help")
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap()
}

/// The man page escapes every hyphen (`\-\-pid\-file`); undo that to compare text.
fn man() -> String {
    read("man/wayinhibit.1").replace("\\-", "-")
}

/// The long options listed under `Options:` in `--help`, without the leading dashes.
fn long_options() -> Vec<String> {
    let help = help();
    let options: Vec<String> = help
        .lines()
        .skip_while(|line| *line != "Options:")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .flat_map(|line| {
            line.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter_map(|word| {
            word.trim_end_matches(',')
                .strip_prefix("--")
                .map(str::to_owned)
        })
        .collect();
    assert!(
        options.len() >= 8,
        "could not read the options from --help: {help}"
    );
    options
}

#[test]
fn every_option_appears_in_every_description_of_the_tool() {
    let readme = read("README.md");
    let man = man();
    let bash = read("completions/wayinhibit.bash");
    let zsh = read("completions/_wayinhibit");
    let fish = read("completions/wayinhibit.fish");

    let mut missing = Vec::new();
    for option in long_options() {
        let long = format!("--{option}");
        for (place, text, needle) in [
            ("README", &readme, long.clone()),
            ("man page", &man, long.clone()),
            ("bash completion", &bash, long.clone()),
            ("zsh completion", &zsh, long.clone()),
            ("fish completion", &fish, format!("-l {option}")),
        ] {
            if !text.contains(&needle) {
                missing.push(format!("{long} is missing from the {place}"));
            }
        }
    }
    assert!(missing.is_empty(), "{missing:#?}");
}

#[test]
fn statements_that_were_once_wrong_are_in_every_place_that_must_carry_them() {
    let help = help();
    let readme = read("README.md");
    let man = man();

    // A command stopped by the timeout or a signal ends with 143, a Ctrl-C on a terminal with 130.
    for (place, text) in [("--help", &help), ("README", &readme), ("man page", &man)] {
        assert!(text.contains("143"), "{place} does not mention status 143");
        assert!(text.contains("130"), "{place} does not mention status 130");
    }

    // Quiet mode silences informational output only.
    for (place, text) in [("--help", &help), ("README", &readme), ("man page", &man)] {
        assert!(
            text.contains("informational"),
            "{place} does not say that --quiet only silences informational output"
        );
    }
    for (place, text) in [
        ("zsh completion", read("completions/_wayinhibit")),
        ("fish completion", read("completions/wayinhibit.fish")),
    ] {
        assert!(
            text.contains("nformational"),
            "{place} still describes --quiet differently"
        );
    }

    // SIGHUP stops the program, unless it was ignored at startup.
    for (place, text) in [("README", &readme), ("man page", &man)] {
        assert!(text.contains("SIGHUP"), "{place} does not mention SIGHUP");
    }

    // GNOME is not covered, and the documentation says so.
    assert!(readme.contains("GNOME"));
    assert!(readme.contains("not covered"));
}

#[test]
fn the_documentation_uses_plain_punctuation() {
    for file in [
        "README.md",
        "man/wayinhibit.1",
        "completions/wayinhibit.bash",
        "completions/_wayinhibit",
        "completions/wayinhibit.fish",
        "CONTRIBUTING.md",
    ] {
        let text = read(file);
        assert!(
            !text.contains('\u{2014}') && !text.contains('\u{2013}'),
            "{file} contains a dash that is not a hyphen"
        );
    }
    let help = help();
    assert!(!help.contains('\u{2014}') && !help.contains('\u{2013}'));
}
