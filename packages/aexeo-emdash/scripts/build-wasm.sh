#!/usr/bin/env bash
set -euo pipefail

# Build the aexeo-emdash-bridge crate into the plugin's wasm/ directory.
#
# As of the merge with origin/main on 2026-04-28, the bridge crate is
# excluded from the cargo workspace because it depends on a
# `seogeo-core` API surface that has been reorganized upstream. See
# crates/aexeo-emdash-bridge/STATUS.md for the porting checklist.
#
# Until that port is complete, this script is a no-op: it verifies
# the pre-built WASM is present in wasm/ (vendored from the v0.1.0
# release) and exits successfully so `npm run build` continues.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PLUGIN_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
WASM="$PLUGIN_DIR/wasm/aexeo_emdash_bridge_bg.wasm"

if [ ! -f "$WASM" ]; then
  echo "error: $WASM is missing." >&2
  echo "  The bridge crate isn't currently in the workspace (see" >&2
  echo "  crates/aexeo-emdash-bridge/STATUS.md). Until it's ported back," >&2
  echo "  the npm package relies on the pre-built WASM that ships with" >&2
  echo "  the v0.1.0 release. If wasm/ was deleted, restore from a" >&2
  echo "  prior commit or from the published tarball:" >&2
  echo "    npm pack @aeptus/aexeo-emdash@0.1.0 && tar xzf *.tgz" >&2
  exit 1
fi

echo "Pre-built WASM present at $WASM (bridge crate port pending; see STATUS.md)"
