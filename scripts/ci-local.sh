#!/bin/sh
set -eu

if [ $# -gt 0 ]; then
    echo "unknown argument: $1" >&2
    exit 2
fi

sh scripts/check-repo.sh
sh scripts/pre-push.sh
