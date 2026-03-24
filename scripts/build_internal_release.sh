#!/bin/sh
set -eu

sh scripts/build-rust.sh

mkdir -p dist
cp target/release/seogeo-cli dist/seogeo-cli
shasum -a 256 dist/seogeo-cli > dist/SHA256SUMS.txt
