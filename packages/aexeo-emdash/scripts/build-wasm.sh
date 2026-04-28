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

# Fix up the .d.ts files to match how WE actually consume the
# generated artifacts. wasm-bindgen's defaults assume bundler-style
# ESM imports of the wasm module (named-exports of WASM functions);
# we use it differently:
#   - The .wasm import is consumed by esbuild's `binary` loader
#     OR our custom inline plugin — in both cases the import resolves
#     to a Uint8Array, not a WebAssembly.Exports.
#   - The bg.js glue isn't shipped with a .d.ts at all by wasm-bindgen,
#     so we provide a minimal one for the public surface we actually
#     call (__wbg_set_wasm, evaluateDocuments, scoreIntelligence).
# Without these overrides, `npm run typecheck` fails.
cat > "$OUT_DIR/aexeo_emdash_bridge_bg.wasm.d.ts" <<'EOF'
// Overrides the wasm-bindgen-generated default to match what
// emdash hosts consuming this package actually receive at the
// import site: a precompiled WebAssembly.Module. Cloudflare
// Workers / workerd disallow runtime WebAssembly.instantiate
// from raw bytes ("Wasm code generation disallowed by embedder"),
// so we depend on the consumer's bundler resolving the import
// to a Module at build time. esbuild's "wasm" loader does this;
// Wrangler does this natively. See scripts/build-wasm.sh for why
// this file is rewritten after every wasm-bindgen run.
declare const wasmModule: WebAssembly.Module;
export default wasmModule;
EOF

cat > "$OUT_DIR/aexeo_emdash_bridge_bg.d.ts" <<'EOF'
// Minimal ambient declarations for the wasm-bindgen "bg" glue
// module. We import this for two purposes:
//   1. The named functions we actually call from JS:
//      __wbg_set_wasm, evaluateDocuments, scoreIntelligence.
//   2. The full namespace, which we hand to WebAssembly.instantiate
//      as the import map — every __wbg_* / __wbindgen_* import the
//      WASM declares needs to resolve at instantiation time.
// We don't enumerate the second set here; they cross the boundary
// only via `import * as glue` and never appear in TypeScript call
// sites by name. See scripts/build-wasm.sh for why this file is
// (re)written after every wasm-bindgen run.
export const __wbg_set_wasm: (wasm: WebAssembly.Exports) => void;
export const evaluateDocuments: (
  documents_json: string,
  config_json?: string | null,
) => string;
export const scoreIntelligence: (documents_json: string) => string;
EOF

echo "Built $OUT_DIR (target=bundler)"
