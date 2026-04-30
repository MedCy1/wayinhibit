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
- stays alive until the process exits

This requires a compositor that supports `zwp_idle_inhibit_manager_v1`.

## Development

```bash
cargo fmt
cargo check
```

Run the inhibitor:

```bash
cargo run
```

## License

This project is licensed under the MIT License.
