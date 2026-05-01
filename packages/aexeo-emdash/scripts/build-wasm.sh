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

# wasm-bindgen 0.2.118 (current pin) does NOT emit a _bg.d.ts for the
# bundler target, so the file at $OUT_DIR/aexeo_emdash_bridge_bg.d.ts is
# hand-maintained and tracked in git. If a future wasm-bindgen version
# starts emitting it, we want to know loudly rather than have it silently
# overwrite the hand-edited declarations. Snapshot the existing file's
# checksum before the run; restore + warn if it changed.
BG_DTS="$OUT_DIR/aexeo_emdash_bridge_bg.d.ts"
BG_DTS_BACKUP=""
if [ -f "$BG_DTS" ]; then
  BG_DTS_BACKUP=$(mktemp)
  cp "$BG_DTS" "$BG_DTS_BACKUP"
fi

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

if [ -n "$BG_DTS_BACKUP" ] && ! cmp -s "$BG_DTS_BACKUP" "$BG_DTS"; then
  echo "warning: wasm-bindgen has begun emitting $BG_DTS for the bundler" >&2
  echo "         target. Restoring the hand-maintained version. Inspect the" >&2
  echo "         new emit (saved as $BG_DTS.wasm-bindgen) and merge any new" >&2
  echo "         exports manually into the tracked file." >&2
  cp "$BG_DTS" "$BG_DTS.wasm-bindgen"
  cp "$BG_DTS_BACKUP" "$BG_DTS"
fi
[ -n "$BG_DTS_BACKUP" ] && rm -f "$BG_DTS_BACKUP"

echo "Built bridge WASM into $OUT_DIR"
