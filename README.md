# wayinhibit

[![CI](https://github.com/MedCy1/wayinhibit/actions/workflows/ci.yml/badge.svg)](https://github.com/MedCy1/wayinhibit/actions/workflows/ci.yml)
[![AUR version](https://img.shields.io/aur/version/wayinhibit?cacheSeconds=3600)](https://aur.archlinux.org/packages/wayinhibit)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`wayinhibit` is a small Wayland idle inhibitor written in Rust.

The goal of the project is simple:

- provide a small and reliable CLI
- stay focused on doing one thing well
- keep the implementation easy to audit and package

## Usage

Run in foreground mode — inhibits idle until you press `Ctrl-C`:

```bash
wayinhibit
```

Run a command under inhibition — idle inhibition is released when the command exits:

```bash
wayinhibit -- sleep 60
wayinhibit -- rsync -av /src /dst
```

Requires a compositor that supports `zwp_idle_inhibit_manager_v1`.

## Development

```bash
cargo fmt
cargo check
```

Common development entrypoints:

```bash
make help
make setup
make quality
make run
make run-command CMD="sleep 10"
```

Run the full local quality suite:

```bash
./scripts/quality.sh all
```

Install the repository Git hooks:

```bash
./scripts/install-hooks.sh
```

The hooks currently run:

- `pre-commit`: `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings`
- `pre-push`: `cargo check --locked` and `cargo test --locked`

Run the inhibitor:

```bash
cargo run
```

Run a command under inhibition:

```bash
cargo run -- -- sleep 10
```

## License

This project is licensed under the MIT License.
