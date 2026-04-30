#!/usr/bin/env sh

set -eu

repo_root=$(git rev-parse --show-toplevel)

git config core.hooksPath "$repo_root/.githooks"

printf '%s\n' "Git hooks installed for $repo_root"
printf '%s\n' "Current hooksPath: $(git config --get core.hooksPath)"
