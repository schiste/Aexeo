// Minimal ambient declarations for the wasm-bindgen "bg" glue
// module. We import this for two purposes:
//   1. The named functions we actually call from JS:
//      __wbg_set_wasm, evaluateDocuments, scoreIntelligence,
//      generateFactsPrompt, validateFactsManifest.
//   2. The full namespace, which we hand to WebAssembly.instantiate
//      as the import map — every __wbg_* / __wbindgen_* import the
//      WASM declares needs to resolve at instantiation time.
// We don't enumerate the second set here; they cross the boundary
// only via `import * as glue` and never appear in TypeScript call
// sites by name. See scripts/build-wasm.sh for why this file is
// hand-maintained: wasm-bindgen does not generate it for our target.
export const __wbg_set_wasm: (wasm: WebAssembly.Exports) => void;
export const evaluateContent: (
  document_json: string,
  preset_name: string,
  sources_json?: string | null,
  evidence_json?: string | null,
  extensions_json?: string | null,
  link_observations_json?: string | null,
) => string;
export const projectLinkDestinations: (document_json: string) => string;
export const projectContentInput: (document_json: string, evidence_json?: string | null) => string;
export const evaluateContentCollection: (
  documents_json: string,
  preset_name: string,
  sources_json?: string | null,
) => string;
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
