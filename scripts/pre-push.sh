#!/bin/sh
set -eu

cargo build --release

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

sh scripts/install-aexeo.sh --from-binary target/release/aexeo-cli --dest-dir "$tmp_dir/bin" >/dev/null
"$tmp_dir/bin/aexeo-cli" --help >/dev/null
