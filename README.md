# wayinhibit

`wayinhibit` is a small Wayland idle inhibitor written in Rust.

The goal of the project is simple:

- provide a small and reliable CLI
- stay focused on doing one thing well
- keep the implementation easy to audit and package

## Status

The repository is currently in bootstrap state.

The initial implementation target is:

- connect to the active Wayland session
- create an idle inhibition request
- keep it alive until the process receives a termination signal

## Current behavior

`wayinhibit` currently:

- connects to the active Wayland compositor
- binds `wl_compositor`
- binds `zwp_idle_inhibit_manager_v1`
- creates a surface and an idle inhibitor
- stays alive until the process receives `Ctrl-C` or `SIGTERM`
- can keep idle inhibition active while a child command is running

This requires a compositor that supports `zwp_idle_inhibit_manager_v1`.

## Development

```bash
cargo fmt
cargo check
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
