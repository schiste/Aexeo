// Minimal ambient declarations for the wasm-bindgen bg.js glue module.
// The full set of `__wbg_*` and `__wbindgen_*` exports exists at runtime
// for the WASM import object to bind against, but we only call the
// public API plus __wbg_set_wasm from TypeScript.
export const __wbg_set_wasm: (wasm: WebAssembly.Exports) => void;
export const evaluateDocuments: (
  documents_json: string,
  config_json?: string | null,
) => string;
export const scoreIntelligence: (documents_json: string) => string;
