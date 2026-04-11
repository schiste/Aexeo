#!/bin/sh
set -eu

cargo clippy --workspace --lib --bins -- \
  -D warnings \
  -D clippy::dbg_macro \
  -D clippy::todo \
  -D clippy::unimplemented \
  -D clippy::unwrap_used \
  -D clippy::expect_used \
  -D clippy::panic \
  -D clippy::panic_in_result_fn
