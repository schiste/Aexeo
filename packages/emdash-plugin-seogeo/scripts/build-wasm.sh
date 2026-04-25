#!/usr/bin/env bash
set -euo pipefail

# Build the aexeo-emdash-bridge crate into the plugin's wasm/ directory.
#
# wasm-pack 0.14.0 currently forwards --artifact-dir to cargo (formerly
# --out-dir, now nightly-only on stable Rust) and aborts with "Expected at
# least one compiler artifact". Until that is fixed upstream, this script
# drives cargo and wasm-bindgen directly. The wasm-bindgen-cli version on
# PATH must match the wasm-bindgen crate in Cargo.lock; install or refresh
# with `cargo install -f wasm-bindgen-cli --version <Cargo.lock version>`.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PLUGIN_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
WORKSPACE_DIR="$(cd "$PLUGIN_DIR/../.." && pwd)"
BRIDGE="aexeo-emdash-bridge"

WANTED=$(awk '/^name = "wasm-bindgen"$/{getline; print; exit}' "$WORKSPACE_DIR/Cargo.lock" \
  | awk -F'"' '{print $2}')
HAVE=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
if [ "$HAVE" != "$WANTED" ]; then
  echo "wasm-bindgen-cli on PATH is ${HAVE:-not installed}; Cargo.lock pins $WANTED." >&2
  echo "Install with: cargo install -f wasm-bindgen-cli --version $WANTED" >&2
  exit 1
fi

cd "$WORKSPACE_DIR"
cargo build --release --target wasm32-unknown-unknown --features wasm -p "$BRIDGE"

WASM_PATH="$WORKSPACE_DIR/target/wasm32-unknown-unknown/release/${BRIDGE//-/_}.wasm"
OUT_DIR="$PLUGIN_DIR/wasm"

rm -rf "$OUT_DIR"
wasm-bindgen "$WASM_PATH" --out-dir "$OUT_DIR" --target bundler
echo "Built $OUT_DIR (target=bundler)"
