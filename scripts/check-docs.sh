#!/bin/sh
set -eu

cargo run -q -p seogeo-cli -- docs check .
cargo run -q -p seogeo-cli -- quality .
