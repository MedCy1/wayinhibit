# Changelog

All notable changes to this project will be documented in this file.
## [Unreleased]

### Bug Fixes

- Use GitHub source tarball for completions and man page
- Sync man page to v0.5.1 and auto-update it on release

### CI & Automation

- Generate release notes with git-cliff

### Documentation

- Set cacheSeconds=3600 on AUR version badge

### Features

- Add cliff.toml for conventional commit changelog
## [0.5.1] - 2026-06-05

### Bug Fixes

- Replace gmail with epitech email, drop email from man page

### Features

- Add man page
- Install man page from PKGBUILD
## [0.5.0] - 2026-06-05

### Bug Fixes

- Trigger AUR update via workflow_run instead of release event
- Make AUR workflow SSH setup and packaging commit robust
- Correct runtime dependencies
- Emit correct depends in generated .SRCINFO

### CI & Automation

- Gate release publication behind Arch Linux smoke test
- Upgrade artifact actions to v5 (Node.js 24)
- Add Wayland integration test with headless weston
- Switch integration test compositor from weston to sway

### Documentation

- Add CI, AUR version, and license badges to README

### Features

- Add workflow_dispatch trigger to AUR workflow for manual runs
- Add aarch64 build and smoke test to release workflow
- Add aarch64 support to PKGBUILD
- Compute aarch64 checksum in AUR workflow
- Add bash, zsh, and fish shell completions
- Install shell completions from PKGBUILD
- Include completion file checksums in AUR workflow
## [0.4.0] - 2026-05-05

### Features

- Add GitHub Actions workflow to auto-update AUR on release
- Add namcap PKGBUILD validation job to CI
- Support --timeout=<value> inline syntax
## [0.3.0] - 2026-05-04

### Bug Fixes

- Replace rust>=1.85 makedepend with cargo to avoid rustup conflict
- Remove cargo makedepend to avoid conflict with rustup
- Switch to pre-built binary in PKGBUILD
- Remove rust cache from release workflow to prevent stale binaries
- Use $CARCH in source URL instead of hardcoded x86_64

### Features

- Add release workflow and switch to binary PKGBUILD
- Add make release script
## [0.2.0] - 2026-05-04

### Bug Fixes

- Derive help text version from CARGO_PKG_VERSION
- Replace libc::signal with sigaction for reliable signal handling

### Documentation

- Replace bootstrap status with usage section

### Features

- Print PID on startup
- Add --quiet / -q flag to suppress output
- Add --timeout / -t flag to stop after a given duration

### Testing

- Add combined --quiet and --timeout test case
## [0.1.0] - 2026-04-30

### CI & Automation

- Add GitHub Actions workflow
- Update checkout action to node 24 runtime

### Documentation

- Document Makefile entrypoints

### Features

- Bootstrap Rust project
- Implement Wayland idle inhibitor
- Handle graceful shutdown
- Support running commands under inhibition

### Testing

- Verify pre-commit hook
