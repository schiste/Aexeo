#!/bin/sh
set -eu

if ! cargo audit --version >/dev/null 2>&1; then
    echo "cargo-audit is required; install it with: cargo install cargo-audit" >&2
    exit 1
fi

if ! cargo deny --version >/dev/null 2>&1; then
    echo "cargo-deny is required; install it with: cargo install cargo-deny" >&2
    exit 1
fi

if ! cargo +nightly udeps --version >/dev/null 2>&1; then
    cat >&2 <<'EOF'
cargo-udeps with the nightly toolchain is required.
Install it with:
  cargo install cargo-udeps
  rustup toolchain install nightly
  rustup component add --toolchain nightly rust-src llvm-tools-preview
EOF
    exit 1
fi

cargo audit
cargo deny check
cargo +nightly udeps --workspace --all-targets
