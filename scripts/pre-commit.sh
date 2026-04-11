#!/bin/sh
set -eu

sh scripts/guard-staged.sh
sh scripts/check-repo.sh
