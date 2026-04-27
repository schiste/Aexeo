// Lazy WASM initializer for the configured plugin path.
//
// In configured mode the plugin runs in the host Worker process,
// where Cloudflare's startup CPU budget (~400ms cold-start) is
// generous enough to compile the 1.2MB seogeo bridge — the budget
// the sandboxed Worker Loader isolate (50ms cpuMs) couldn't fit.
//
// We lazy-initialize on first call rather than at module load:
//   - The plugin module loads instantly (no top-level await), so
//     emdash's integration setup isn't slowed down at startup.
//   - The first Refresh / first afterSave eats ~80-150ms compile;
//     all subsequent calls are microseconds (cached module).
//   - If a host never invokes our handlers, no WASM compile cost.
//
// The .wasm bytes are base64-inlined into the bundle by esbuild's
// `binary` loader (see scripts/build-bundle.mjs configured target),
// so this module doesn't need to know how to read a file at runtime.

import * as glue from "../wasm/aexeo_emdash_bridge_bg.js";
// esbuild's binary loader resolves a .wasm import to a Uint8Array
// at bundle time. The configured-bundle build sets this loader
// explicitly; for the sandboxed bundle path this file isn't used.
import wasmBytes from "../wasm/aexeo_emdash_bridge_bg.wasm";

let initPromise: Promise<void> | null = null;

async function ensureInitialized(): Promise<void> {
  if (initPromise !== null) {
    return initPromise;
  }
  initPromise = (async () => {
    const { instance } = await WebAssembly.instantiate(
      wasmBytes as unknown as BufferSource,
      {
        "./aexeo_emdash_bridge_bg.js":
          glue as unknown as WebAssembly.ModuleImports,
      },
    );
    glue.__wbg_set_wasm(instance.exports as unknown as WebAssembly.Exports);
    (instance.exports as { __wbindgen_start: () => void }).__wbindgen_start();
  })();
  return initPromise;
}

export async function evaluateDocuments(
  documentsJson: string,
  configJson?: string | null,
): Promise<string> {
  await ensureInitialized();
  return glue.evaluateDocuments(documentsJson, configJson);
}

export async function scoreIntelligence(
  documentsJson: string,
): Promise<string> {
  await ensureInitialized();
  return glue.scoreIntelligence(documentsJson);
}
