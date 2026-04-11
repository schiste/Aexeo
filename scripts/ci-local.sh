#!/bin/sh
set -eu

run_audit=0

while [ $# -gt 0 ]; do
    case "$1" in
        --with-audit)
            run_audit=1
            shift
            ;;
        *)
            echo "unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

sh scripts/check-repo.sh
sh scripts/pre-push.sh

if [ "$run_audit" -eq 1 ]; then
    if cargo audit --version >/dev/null 2>&1; then
        cargo audit
    else
        echo "cargo audit is not installed; skipping dependency audit" >&2
    fi
fi
