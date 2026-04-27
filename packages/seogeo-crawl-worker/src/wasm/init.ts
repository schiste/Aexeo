// Initialize the wasm-bindgen glue against the WASM module Cloudflare
// hands us via the BRIDGE_WASM env binding. Wrangler ships the .wasm
// file as a precompiled WebAssembly.Module, so we can do a synchronous
// `new WebAssembly.Instance(...)` instead of the async instantiate
// call the sandbox bundle has to use. This avoids the cpuMs/TLA issues
// the sandbox runs into; the Worker isolate compiles the module once
// at deploy time and reuses the compiled artifact across requests.

import * as glue from "./aexeo_emdash_bridge_bg.js";

let initialized = false;

export function ensureInitialized(module: WebAssembly.Module): void {
  if (initialized) {
    return;
  }
  const instance = new WebAssembly.Instance(module, {
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
