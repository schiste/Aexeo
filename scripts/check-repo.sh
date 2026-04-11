#!/bin/sh
set -eu

sh scripts/check-code.sh
sh scripts/check-deps.sh
sh scripts/check-docs.sh
sh scripts/check-config.sh
