#!/bin/sh
set -eu

cargo build --release

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

sh scripts/install-seogeo.sh --from-binary target/release/seogeo-cli --dest-dir "$tmp_dir/bin" >/dev/null
"$tmp_dir/bin/seogeo-cli" --help >/dev/null
