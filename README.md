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

## Development

```bash
cargo fmt
cargo check
```

## License

This project is licensed under the MIT License.
