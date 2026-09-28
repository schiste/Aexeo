// Minimal ambient declarations for the wasm-bindgen bg.js glue module.
// The full set of `__wbg_*` and `__wbindgen_*` exports exists at runtime
// for the WASM import object to bind against, but we only call the
// public API plus __wbg_set_wasm from TypeScript.
//
// Hand-maintained and tracked in git, unlike the .wasm and _bg.js next
// to it: wasm-bindgen 0.2.118 does not emit a _bg.d.ts for the bundler
// target, so nothing regenerates this file and there is no way for it to
// drift out of sync silently — but it does mean it has to be edited by
// hand when the bridge surface changes. It must stay identical in shape
// to ../../aexeo-emdash/wasm/aexeo_emdash_bridge_bg.d.ts; copy that
// file here whenever the bridge grows or drops an export.
export const __wbg_set_wasm: (wasm: WebAssembly.Exports) => void;
export const evaluateDocuments: (
  documents_json: string,
  config_json?: string | null,
) => string;
export const scoreIntelligence: (
  documents_json: string,
  manifest_json?: string | null,
) => string;
export const generateFactsPrompt: (documents_json: string) => string;
export const validateFactsManifest: (
  manifest_json: string,
  documents_json: string,
) => string;
