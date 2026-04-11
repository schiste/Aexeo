#!/bin/sh
set -eu

cargo run -q -p seogeo-cli -- config print . --format json >/dev/null
cargo run -q -p seogeo-cli -- config print . --config docs/examples/seogeo.v1.toml --format json >/dev/null
