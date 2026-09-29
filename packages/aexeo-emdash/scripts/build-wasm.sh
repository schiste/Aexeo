#!/usr/bin/env bash
set -euo pipefail

# Build the Aexeo WASM bridge and run wasm-bindgen over it.
#
# Usage: build-wasm.sh [OUT_DIR]
#
#   OUT_DIR  Directory for the wasm-bindgen output. Defaults to the
#            plugin's own `wasm/`. packages/aexeo-crawl-worker passes its
#            `src/wasm/` so both packages come out of one cargo +
#            wasm-bindgen invocation and cannot drift apart.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PLUGIN_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROOT_DIR="$(cd "$PLUGIN_DIR/../.." && pwd)"
OUT_DIR="${1:-$PLUGIN_DIR/wasm}"
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
#
# A second file in $OUT_DIR is also tracked, for a different reason:
# aexeo_emdash_bridge.d.ts is the module `src/evaluator.ts` imports, so it is
# the one `tsc` resolves. The CI typecheck job has no Rust toolchain and
# deliberately does not run `build:wasm`, so an untracked copy means TS2307 on
# every fresh checkout. It is generated, but it is what the typecheck validates
# against, so it is snapshotted too -- with a hard failure rather than a
# warning, because unlike the hand-maintained file a change here means the Rust
# exports moved and the tracked copy is now stale.
BG_DTS="$OUT_DIR/aexeo_emdash_bridge_bg.d.ts"
API_DTS="$OUT_DIR/aexeo_emdash_bridge.d.ts"
BG_DTS_BACKUP=""
API_DTS_BACKUP=""
if [ -f "$BG_DTS" ]; then
  BG_DTS_BACKUP=$(mktemp)
  cp "$BG_DTS" "$BG_DTS_BACKUP"
fi
if [ -f "$API_DTS" ]; then
  API_DTS_BACKUP=$(mktemp)
  cp "$API_DTS" "$API_DTS_BACKUP"
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

# The API declarations are generated, so a change is expected when the Rust
# exports change. But the tracked copy is what `tsc` checks the plugin
# against, so silently accepting the new one would leave the typecheck
# validating a signature the shipped WASM no longer has. Restore the tracked
# copy, keep the new one for review, and fail the build.
if [ -n "$API_DTS_BACKUP" ] && ! cmp -s "$API_DTS_BACKUP" "$API_DTS"; then
  cp "$API_DTS" "$API_DTS.wasm-bindgen"
  cp "$API_DTS_BACKUP" "$API_DTS"
  echo "error: $API_DTS changed because the bridge's exported surface" >&2
  echo "       changed. The tracked copy has been restored so the typecheck" >&2
  echo "       cannot drift from it; the newly generated version is at" >&2
  echo "       $API_DTS.wasm-bindgen. Review the diff, then commit the" >&2
  echo "       updated tracked file." >&2
  exit 1
fi
[ -n "$API_DTS_BACKUP" ] && rm -f "$API_DTS_BACKUP"

echo "Built bridge WASM into $OUT_DIR"
