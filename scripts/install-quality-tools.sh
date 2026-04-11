#!/bin/sh
set -eu

cargo install cargo-audit cargo-deny cargo-udeps
rustup toolchain install nightly
rustup component add --toolchain nightly rust-src llvm-tools-preview
