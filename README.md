# wayinhibit

[![CI](https://github.com/MedCy1/wayinhibit/actions/workflows/ci.yml/badge.svg)](https://github.com/MedCy1/wayinhibit/actions/workflows/ci.yml)
[![Coverage](https://codecov.io/github/MedCy1/wayinhibit/graph/badge.svg?token=KZAB4UMZN6)](https://codecov.io/github/MedCy1/wayinhibit)
[![AUR version](https://img.shields.io/aur/version/wayinhibit?cacheSeconds=3600)](https://aur.archlinux.org/packages/wayinhibit)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A small Wayland idle inhibitor written in Rust. Prevents your compositor from locking the screen or suspending while a command is running or until you stop it manually.

## Features

- Foreground mode: inhibits idle until `Ctrl-C`, `SIGTERM` or `SIGHUP`
- Command mode: wraps a command and releases the inhibition when it exits
- Timeout: stops automatically after a given duration (`30s`, `5m`, `2h`)
- Quiet mode: suppresses informational output, for use in scripts (warnings and errors still go to stderr)
- Exit code propagation: in command mode, exits with the same code as the wrapped command
- PID file: writes the PID to a file so scripts or status bars can signal the process
- Hooks: runs shell commands when inhibition starts and stops
- Dry-run: tests hooks and scripts without a live compositor
- Toggle: starts or stops from a single keybind or status bar click

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

Shell completions (bash, zsh, fish) are in `completions/` and the man page is `man/wayinhibit.1`. The AUR package installs both; with `cargo install` or a manual build you can copy them yourself.

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

In command mode the timeout stops the command as well: `SIGTERM` goes to its whole process group, then `SIGKILL` after 2 seconds. The exit status is then 143 (or 137 if the command had to be killed).

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

Write the PID to a file so other scripts can signal it:

```bash
wayinhibit --quiet --pid-file /tmp/wayinhibit.pid &
kill "$(cat /tmp/wayinhibit.pid)"
```

While wayinhibit runs, the file is locked, so it always names the instance that owns it. A second instance started with the same path still does its job, but it prints a warning and leaves the file alone. The lock disappears when the process ends, even after a crash, so a leftover file never blocks the next start.

Keep the file on a local filesystem such as `/tmp` or `$XDG_RUNTIME_DIR`. Where locking is unavailable, wayinhibit warns and falls back to a best-effort check, which can misjudge when two instances start at the same moment.

Run shell commands when inhibition starts and stops:

```bash
wayinhibit \
  --on-inhibit  "notify-send wayinhibit 'Idle inhibited'" \
  --on-release  "notify-send wayinhibit 'Idle released'"
```

A hook runs in its own process group and its exit status is ignored. `--on-release` always follows `--on-inhibit`, also when the command cannot be started. A request to stop ends a slow `--on-inhibit` hook; `--on-release` is not cut short by the request that made wayinhibit stop, only by a second one.

Test your hooks without a live compositor:

```bash
wayinhibit --dry-run --timeout 5s \
  --on-inhibit "echo start" \
  --on-release "echo stop"
```

Toggle inhibition from a single command (it starts if nothing is running and stops the running instance otherwise), for example from a keybind or a status bar click handler:

```bash
wayinhibit --quiet --toggle --pid-file /tmp/wayinhibit.pid
```

## Options

| Flag | Description |
|---|---|
| `-t`, `--timeout <DURATION>` | Stop after a given duration (e.g. `30s`, `5m`, `2h`). In command mode the command is stopped too |
| `-q`, `--quiet` | Suppress informational output (warnings and errors remain) |
| `-p`, `--pid-file <PATH>` | Write PID to `PATH` on startup and lock it, remove it on exit. The lock means the file always names its owner |
| `--on-inhibit <CMD>` | Run `CMD` (via `sh -c`) when inhibition starts |
| `--on-release <CMD>` | Run `CMD` (via `sh -c`) when inhibition stops |
| `--dry-run` | Run without connecting to Wayland (for testing hooks) |
| `--toggle` | Stop the running instance from `--pid-file`, or start one |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

## Exit status

| Code | Meaning |
|---|---|
| `0` | Stopped normally: `SIGINT`, `SIGTERM` or `SIGHUP`, the timeout elapsed (foreground mode), or the wrapped command exited with 0 |
| `1` | Runtime error, for example no Wayland compositor to connect to, or the connection lost in foreground mode |
| `2` | Invalid arguments |
| *N* | Command mode: the wrapped command's exit code, or 128 + *n* if it was killed by signal *n* |

In command mode, `--timeout` or a signal sent to wayinhibit stops the command with `SIGTERM`, which usually gives 143. When wayinhibit runs on a terminal it hands the terminal to the command, so that interactive programs work: `Ctrl-C` then interrupts the command itself (usually 130) and `Ctrl-Z` suspends the whole job. A hangup that was ignored when wayinhibit started (as `nohup` does) stays ignored.

## Waybar integration

**Option 1: Waybar's built-in module** (no wayinhibit required):

```json
"idle_inhibitor": {
    "format": "{icon}",
    "format-icons": {
        "activated": "󰒲",
        "deactivated": "󰒳"
    }
}
```

**Option 2: custom module using wayinhibit** (supports hooks, pid-file, timeout):

```json
"custom/idle-inhibitor": {
    "exec": "[ -f /tmp/wayinhibit.pid ] && echo '󰒲 inhibiting' || echo '󰒳 idle'",
    "interval": 2,
    "on-click": "wayinhibit --quiet --toggle --pid-file /tmp/wayinhibit.pid &",
    "tooltip": false
}
```

Use option 2 if you want `--on-inhibit`/`--on-release` hooks, a timeout, or need to drive wayinhibit from other scripts as well.

The module reads the PID file, which wayinhibit removes on a normal exit. After a crash or `kill -9` the file can outlive the process and the module keeps showing "inhibiting"; the next click starts a fresh instance and puts it right.

## Keybinds and services

Sway:

```
bindsym $mod+i exec wayinhibit --quiet --toggle --pid-file $XDG_RUNTIME_DIR/wayinhibit.pid
```

Hyprland (`hyprland.conf` syntax):

```
bind = $mainMod, I, exec, wayinhibit --quiet --toggle --pid-file $XDG_RUNTIME_DIR/wayinhibit.pid
```

A systemd user service, started and stopped by hand with `systemctl --user start wayinhibit` (do not enable it, it would inhibit idle for the whole session):

```ini
[Unit]
Description=Keep the screen awake

[Service]
ExecStart=/usr/bin/wayinhibit --quiet
```

The service needs `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` in its environment. Most compositors import them into the user manager; if yours does not, run `systemctl --user import-environment WAYLAND_DISPLAY` from its startup file.

## Troubleshooting

- **`failed to connect to the Wayland compositor`**: `WAYLAND_DISPLAY` or `XDG_RUNTIME_DIR` is missing, which is usual over SSH or in a service. Check `echo $WAYLAND_DISPLAY`.
- **The screen still locks or dims**: read the table in the next section. On GNOME wayinhibit cannot work and prints a warning at startup. Elsewhere, check that your idle daemon honors inhibitors: one configured to ignore them locks the screen anyway.
- **Started after the session was already idle**: the protocol does not wake an idle session, and an inhibitor created at that moment is not honored until the next idle period. Start wayinhibit before you step away.
- **Lid close and manual suspend**: not covered, see the end of the next section.
- **`PID file ... is already in use`**: another wayinhibit owns the file. Use `--toggle` to stop it, or another path.

## Compatibility

Requires a compositor that supports the `zwp_idle_inhibit_manager_v1` Wayland protocol, which includes:

- **wlroots-based**: Sway, Hyprland, river, labwc, Wayfire, Cage, niri
- **GNOME** (Mutter, Wayland session)
- **KDE Plasma** (KWin, Wayland session)
- **Cinnamon** (Muffin, Wayland session)
- **COSMIC**, Mir, Louvre, phoc, Jay, Treeland

Compositors disagree on which surface an idle inhibitor needs. Sway 1.11 and later and KDE Plasma 6 ignore an inhibitor on a surface that is not shown, while Hyprland, labwc, Wayfire and Cage still honor one on a bare surface. wayinhibit therefore asks for two inhibitions at the same time: one on a bare surface, and one on a transparent 1x1 pixel shown in the background layer with `wlr-layer-shell`. The second one is created about 300 ms after the pixel appears, because Hyprland judges an inhibitor once, when it is created.

How well each compositor is covered:

| Compositor | Evidence |
|---|---|
| Sway 1.12 | automated test, run in CI |
| KDE Plasma (KWin 6.8) | automated test, run locally when `kwin_wayland` is installed |
| Hyprland 0.57 | source reading, plus a manual check by the maintainer |
| river, labwc, Wayfire, Cage, niri, COSMIC | source reading only, not tested |
| GNOME (Mutter) | **not covered**: it only honors real windows and has no `wlr-layer-shell`; wayinhibit warns on startup |

`wayinhibit` only inhibits the compositor's idle state (screen lock, screen off). It does not prevent `systemd-logind` from suspending the system on events it handles itself, such as closing a laptop lid.

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
make e2e      # Run end-to-end tests against a real headless Sway (requires `sway`)
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for more details.

## License

MIT. See [`LICENSE`](LICENSE).
