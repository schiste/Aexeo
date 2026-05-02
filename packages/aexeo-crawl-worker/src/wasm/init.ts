// Initialize the wasm-bindgen glue against the WASM module Wrangler
// bundles in via the direct .wasm import below. Modules-based Workers
// (ESM) resolve `import x from "./foo.wasm"` to a precompiled
// WebAssembly.Module at build time, so we get synchronous
// `new WebAssembly.Instance(...)` and no runtime compile cost. This
// avoids the cpuMs/TLA issues the emdash sandbox runs into and is
// also why the older [wasm_modules] wrangler.toml binding is not
// used: that syntax is for service-worker (non-modules) scripts.

import bridgeModule from "./aexeo_emdash_bridge_bg.wasm";
import * as glue from "./aexeo_emdash_bridge_bg.js";

let initialized = false;

export function ensureInitialized(): void {
  if (initialized) {
    return;
  }
  const instance = new WebAssembly.Instance(bridgeModule, {
    "./aexeo_emdash_bridge_bg.js": glue,
  });
  glue.__wbg_set_wasm(instance.exports);
  // wasm-bindgen factors out the externref table init into a wasm
  // export; calling it once after binding `wasm` is what every
  // wasm-bindgen target shape does (web/bundler/nodejs).
  (instance.exports as { __wbindgen_start: () => void }).__wbindgen_start();
  initialized = true;
}

export const evaluateDocuments = glue.evaluateDocuments;
export const scoreIntelligence = glue.scoreIntelligence;
