# Contributing

## Prerequisites

- Rust stable toolchain
- A Wayland compositor that supports `zwp_idle_inhibit_manager_v1` for manual testing

## Setup

Install the repository Git hooks:

```bash
make setup
```

The hooks run formatting and Clippy checks on commit, and compilation and tests on push.

## Development

```bash
make quality       # Run the full quality suite
make run           # Run wayinhibit in foreground mode
make run-command CMD="sleep 5"  # Run a command under inhibition
```

## Commit conventions

This project uses [Conventional Commits](https://www.conventionalcommits.org/).
The changelog is generated automatically from commit messages, so please follow the format:

```
<type>: <description>

Types: feat, fix, ci, docs, test, refactor, chore
```

Examples:

```
feat: add --daemon flag to run in background
fix: handle SIGPIPE gracefully
ci: cache Cargo registry in CI workflow
```

## Submitting changes

Open a pull request against `main`. CI must pass before merging:

- `cargo fmt --check`
- `cargo clippy --locked --all-targets -- -D warnings`
- `cargo test --locked`
- Wayland integration test (headless sway compositor)
- PKGBUILD validation with namcap
