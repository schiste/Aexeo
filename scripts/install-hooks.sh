#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)

git -C "$repo_root" config core.hooksPath .githooks
chmod 0755 "$repo_root/.githooks/pre-commit" "$repo_root/.githooks/pre-push"

printf '%s\n' "$repo_root/.githooks"
