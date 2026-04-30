#!/usr/bin/env sh

set -eu

mode="${1:-all}"

run() {
  printf '\n==> %s\n' "$1"
  shift
  "$@"
}

case "$mode" in
  pre-commit)
    run "Checking formatting" cargo fmt --check
    run "Running Clippy" cargo clippy --locked --all-targets -- -D warnings
    ;;
  pre-push)
    run "Checking compilation" cargo check --locked
    run "Running tests" cargo test --locked
    ;;
  all)
    run "Checking formatting" cargo fmt --check
    run "Checking compilation" cargo check --locked
    run "Running tests" cargo test --locked
    run "Running Clippy" cargo clippy --locked --all-targets -- -D warnings
    ;;
  *)
    printf '%s\n' "Unknown mode: $mode" >&2
    printf '%s\n' "Usage: scripts/quality.sh [pre-commit|pre-push|all]" >&2
    exit 2
    ;;
esac
