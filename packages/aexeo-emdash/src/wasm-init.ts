// Lazy WASM initializer for the configured plugin path.
//
// Cloudflare Workers (and the workerd runtime that @astrojs/cloudflare
// uses for dev) disallow `WebAssembly.instantiate(bytes)` from raw
// bytes at runtime — the embedder rejects "Wasm code generation".
// The only allowed path is to import the .wasm file directly so the
// bundler resolves it to a precompiled WebAssembly.Module at build
// time; from there `new WebAssembly.Instance(module, imports)` is
// permitted (synchronous, no compilation needed).
//
// We lazy-initialize on first call rather than at module load:
//   - Plugin module loads instantly (no top-level await), so emdash's
//     integration setup isn't slowed down at startup.
//   - First Refresh / first afterSave runs the synchronous Instance()
//     constructor — microseconds, since the Module is precompiled.
//   - If a host never invokes our handlers, zero cost.

import * as glue from "../wasm/aexeo_emdash_bridge_bg.js";
// Bundler-resolved import: emdash hosts using @astrojs/cloudflare
// must support `.wasm` imports as precompiled WebAssembly.Module
// instances. esbuild does this natively when `loader: { ".wasm":
// "wasm" }`; Wrangler does this via the `compatibility_flags` line
// most Cloudflare Worker projects already have. If a consumer's
// build chain doesn't resolve this import correctly, the
// lazy-instantiated initializer below will throw at first call
// with a clear error.
import bridgeModule from "../wasm/aexeo_emdash_bridge_bg.wasm";

let initialized = false;

function ensureInitialized(): void {
  if (initialized) {
    return;
  }
  if (!(bridgeModule instanceof WebAssembly.Module)) {
    throw new Error(
      "aexeo: WASM module did not resolve to a WebAssembly.Module — " +
        "the consumer's bundler isn't configured for .wasm imports. " +
        "Add the .wasm loader to your Vite/wrangler config; see " +
        "@aeptus/aexeo-emdash INSTALL.md for the recipe.",
    );
  }
  const instance = new WebAssembly.Instance(bridgeModule, {
    "./aexeo_emdash_bridge_bg.js":
      glue as unknown as WebAssembly.ModuleImports,
  });
  glue.__wbg_set_wasm(instance.exports as unknown as WebAssembly.Exports);
  (instance.exports as { __wbindgen_start: () => void }).__wbindgen_start();
  initialized = true;
}

export async function evaluateDocuments(
  documentsJson: string,
  configJson?: string | null,
): Promise<string> {
  ensureInitialized();
  return glue.evaluateDocuments(documentsJson, configJson);
}

export async function scoreIntelligence(
  documentsJson: string,
  manifestJson?: string | null,
): Promise<string> {
  ensureInitialized();
  return glue.scoreIntelligence(documentsJson, manifestJson ?? undefined);
}

// Renders the LLM authoring prompt for the editor's site. Pure projection
// over the documents — no KV, no host filesystem.
export async function generateFactsPrompt(
  documentsJson: string,
): Promise<string> {
  ensureInitialized();
  return glue.generateFactsPrompt(documentsJson);
}

// Validates a candidate facts.json against the site documents. Returns the
// raw JSON string of { validation, assessment } so the caller can render
// either inline or hand it to the React component verbatim.
export async function validateFactsManifest(
  manifestJson: string,
  documentsJson: string,
): Promise<string> {
  ensureInitialized();
  return glue.validateFactsManifest(manifestJson, documentsJson);
}
