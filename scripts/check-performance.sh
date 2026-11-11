#!/bin/sh
set -eu

node --test scripts/test-check-performance-budget.js

STATIC_JSON=$(cargo run --release -q -p aexeo-core --example bench_static_audit -- --json 5)
RUNTIME_JSON=$(cargo run --release -q -p aexeo-core --example bench_runtime_audit -- --json 5)
CONTENT_JSON=$(cargo run --release -q -p aexeo-core --example content_bench -- --json)
LINK_GRAPH_JSON=$(cargo run --release -q -p aexeo-core --example link_graph_bench -- --json 20)
EDITORIAL_JSON=$(cargo run --release -q -p aexeo-core --example editorial_bench -- --json 20)
EMDASH_JSON=$(cargo run --release -q -p aexeo-emdash-bridge --example emdash_content_bench)
cargo build --release -q -p aexeo-cli
STARTUP_JSON=$(node scripts/bench-content-startup.js)

mkdir -p .aexeo-reports
printf '%s\n%s\n%s\n%s\n%s\n%s\n%s\n' "$STATIC_JSON" "$RUNTIME_JSON" "$CONTENT_JSON" "$LINK_GRAPH_JSON" "$EDITORIAL_JSON" "$STARTUP_JSON" "$EMDASH_JSON" | node scripts/check-performance-budget.js
