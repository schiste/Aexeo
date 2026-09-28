#!/bin/sh
set -eu

if ! git rev-parse --git-dir >/dev/null 2>&1; then
    exit 0
fi

if [ -z "$(git diff --cached --name-only --diff-filter=ACMR)" ]; then
    exit 0
fi

# rg exits 2 when it cannot be executed, which is indistinguishable from
# exit 1 (no match) inside the `if` conditions below. Without this guard the
# secret and TODO/FIXME scans silently pass on a machine with no ripgrep.
# Fail closed instead: this script is the only thing standing between a
# committed credential and the remote.
if ! command -v rg >/dev/null 2>&1; then
    echo "ripgrep (rg) is required by scripts/guard-staged.sh but was not found on PATH." >&2
    echo "  macOS:          brew install ripgrep" >&2
    echo "  Debian/Ubuntu:  apt-get install -y ripgrep" >&2
    echo "  or run:         sh scripts/install-quality-tools.sh" >&2
    echo "Refusing to run the staged-diff scan: it would pass without ever searching." >&2
    exit 1
fi

git diff --cached --check

diff_output=$(git diff --cached -U0 --no-color -- . ':(exclude)scripts/guard-staged.sh')

if printf '%s\n' "$diff_output" | rg -n '^\+.*(BEGIN [A-Z ]*PRIVATE KEY|ghp_[A-Za-z0-9_]+|AKIA[0-9A-Z]{16}|OPENAI_API_KEY|sk-[A-Za-z0-9]{20,}|xox[baprs]-)' >/dev/null; then
    echo "staged diff appears to contain a secret or credential-like token" >&2
    exit 1
fi

git diff --cached --name-only --diff-filter=ACMR | while IFS= read -r path; do
    [ -n "$path" ] || continue
    case "$path" in
        *.rs|*.sh)
            if [ "$path" = "scripts/guard-staged.sh" ]; then
                continue
            fi
            diff_for_path=$(git diff --cached -U0 --no-color -- "$path")
            if printf '%s\n' "$diff_for_path" | rg -n '^\+.*\b(TODO|FIXME)\b' >/dev/null; then
                echo "staged file contains TODO or FIXME markers: $path" >&2
                exit 1
            fi
            ;;
    esac
done
