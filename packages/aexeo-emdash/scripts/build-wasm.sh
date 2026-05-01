#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PLUGIN_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROOT_DIR="$(cd "$PLUGIN_DIR/../.." && pwd)"
OUT_DIR="$PLUGIN_DIR/wasm"
TARGET_DIR="$ROOT_DIR/target/wasm32-unknown-unknown/release"
WASM_NAME="aexeo_emdash_bridge"
RAW_WASM="$TARGET_DIR/${WASM_NAME}.wasm"

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "error: wasm-bindgen CLI is required to build @aeptus/aexeo-emdash" >&2
  echo "install it with: cargo install wasm-bindgen-cli" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"

cargo build \
  --manifest-path "$ROOT_DIR/Cargo.toml" \
  -p aexeo-emdash-bridge \
  --release \
  --target wasm32-unknown-unknown \
  --features wasm

wasm-bindgen \
  --target bundler \
  --out-dir "$OUT_DIR" \
  "$RAW_WASM"

echo "Built bridge WASM into $OUT_DIR"
