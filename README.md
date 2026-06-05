# wayinhibit

[![CI](https://github.com/MedCy1/wayinhibit/actions/workflows/ci.yml/badge.svg)](https://github.com/MedCy1/wayinhibit/actions/workflows/ci.yml)
[![AUR version](https://img.shields.io/aur/version/wayinhibit?cacheSeconds=3600)](https://aur.archlinux.org/packages/wayinhibit)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A small Wayland idle inhibitor written in Rust. Prevents your compositor from locking the screen or suspending while a command is running or until you stop it manually.

## Features

- **Foreground mode** — inhibits idle until `Ctrl-C` or `SIGTERM`
- **Command mode** — wraps a command and releases inhibition when it exits
- **Timeout** — stop automatically after a given duration (`30s`, `5m`, `2h`)
- **Quiet mode** — suppress all output for use in scripts
- **Exit code propagation** — in command mode, exits with the same code as the wrapped command
- **PID printed on startup** — makes it easy to signal from other scripts

## Installation

**Arch Linux (AUR)**

```bash
yay -S wayinhibit
# or: paru -S wayinhibit
```

**Cargo**

```bash
cargo install wayinhibit
```

**Nix**

```bash
nix run github:MedCy1/wayinhibit
```

**Build from source**

```bash
git clone https://github.com/MedCy1/wayinhibit
cd wayinhibit
cargo build --release
```

## Usage

Inhibit idle until `Ctrl-C`:

```bash
wayinhibit
```

Inhibit idle while a command runs, then exit with its exit code:

```bash
wayinhibit -- rsync -av /src /dst
wayinhibit -- yt-dlp https://example.com/video
wayinhibit -- ssh user@host 'long-running-task'
```

Stop automatically after a timeout:

```bash
wayinhibit --timeout 2h
wayinhibit --timeout 30m -- ./backup.sh
```

Use in a script without any output:

```bash
wayinhibit --quiet -- ./encode.sh
```

Signal from another terminal using the printed PID:

```bash
# terminal 1
wayinhibit
# Inhibiting idle. PID: 12345. Press Ctrl-C to stop.

# terminal 2
kill 12345
```

## Options

| Flag | Description |
|---|---|
| `-t`, `--timeout <DURATION>` | Stop after a given duration (`30s`, `5m`, `2h`) |
| `-q`, `--quiet` | Suppress all output |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

## Compatibility

Requires a compositor that supports the `zwp_idle_inhibit_manager_v1` Wayland protocol, which includes:

- **wlroots-based**: Sway, Hyprland, river, labwc, wayfire
- **GNOME** (Wayland session)
- **KDE Plasma** (Wayland session)

## Development

Install the repository Git hooks:

```bash
make setup
```

Run the full quality suite:

```bash
make quality
```

Individual checks:

```bash
make fmt      # Check formatting
make clippy   # Run Clippy
make test     # Run tests
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for more details.

## License

MIT — see [`LICENSE`](LICENSE).
