#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-}"

if [ -z "$VERSION" ]; then
    printf 'Usage: make release VERSION=x.y.z\n' >&2
    exit 2
fi

if ! printf '%s' "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    printf 'error: VERSION must be in the form x.y.z (e.g. 1.2.3)\n' >&2
    exit 2
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
    printf 'error: working tree is not clean — commit or stash your changes first\n' >&2
    exit 1
fi

CURRENT=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')

if [ "$VERSION" = "$CURRENT" ]; then
    printf 'error: VERSION %s is already the current version\n' "$VERSION" >&2
    exit 1
fi

printf '==> Releasing %s -> %s\n' "$CURRENT" "$VERSION"

printf '==> Running quality checks...\n'
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked

printf '==> Bumping version in Cargo.toml...\n'
sed -i "s/^version = \"$CURRENT\"/version = \"$VERSION\"/" Cargo.toml
cargo check

printf '==> Committing...\n'
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to $VERSION"

printf '==> Tagging and pushing...\n'
git tag "v$VERSION"
git push
git push origin "v$VERSION"

printf '==> Done. GitHub Actions will build the binary and create the release.\n'
