#!/bin/sh
set -eu

cargo run --release -p seogeo-core --example bench_static_audit -- 10
cargo run --release -p seogeo-core --example bench_runtime_audit -- 10
