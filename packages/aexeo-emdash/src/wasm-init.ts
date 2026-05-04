// Lazy WASM initializer for the configured plugin path.
//
// Three runtimes need to load the bridge .wasm and they have
// incompatible constraints:
//
//   1. Cloudflare Workers / workerd (production target):
//      - `WebAssembly.instantiate(bytes)` and `WebAssembly.compile(bytes)`
//        are forbidden by the embedder ("Wasm code generation
//        disallowed").
//      - The only allowed path is to use a precompiled
//        WebAssembly.Module that the bundler resolved at build time,
//        then `new WebAssembly.Instance(module, imports)` synchronously.
//      - Both static and dynamic `import`s of `.wasm` files yield
//        `{ default: WebAssembly.Module }` in this runtime.
//
//   2. Node ESM (Astro dev, Vite SSR runner, plain Node hosts):
//      - Static `import x from "./foo.wasm"` rejects with SyntaxError
//        because Node's WASM ESM integration doesn't expose a default
//        export. This is what broke @aeptus/aexeo-emdash@0.8.7 in
//        `pnpm dev` for Aeptus on Astro 6.1.3 + Vite 7.3.1.
//      - `WebAssembly.compile(bytes)` is fine in Node.
//      - We can read bytes from disk via `node:fs/promises`.
//
//   3. Browser-like edge runtimes:
//      - `fetch` + `WebAssembly.compileStreaming` works.
//      - Rare in our deployment matrix; kept as the last fallback.
//
// The previous version relied on a single static
// `import bridgeModule from "./...wasm"` that Cloudflare resolved
// to a Module but Node's ESM rejected at parse time. Replaced with
// a runtime-detected loader that tries the bundler path first and
// falls back to fs-based compilation when the bundler path doesn't
// produce a Module.
//
// Lazy-initialized on first call rather than at module load, so
// emdash's integration setup isn't slowed down by WASM compile.
// Subsequent calls hit the cached Module.

import * as glue from "../wasm/aexeo_emdash_bridge_bg.js";

const WASM_SPECIFIER = "../wasm/aexeo_emdash_bridge_bg.wasm";

let initialized = false;
let cachedModule: WebAssembly.Module | null = null;

async function loadBridgeModule(): Promise<WebAssembly.Module> {
  if (cachedModule !== null) {
    return cachedModule;
  }

  // Path A: bundler-resolved dynamic import. In Cloudflare Workers,
  // Wrangler resolves both static and dynamic `.wasm` imports to a
  // precompiled `WebAssembly.Module` exposed as `default`. In Node
  // ESM (Astro dev) this rejects at await time with a SyntaxError;
  // the catch falls through to Path B.
  try {
    const mod = (await import(WASM_SPECIFIER)) as { default?: unknown };
    if (mod.default instanceof WebAssembly.Module) {
      cachedModule = mod.default;
      return cachedModule;
    }
  } catch {
    // Fall through. The error itself isn't useful — Node throws a
    // SyntaxError and Vite throws transformation errors; the
    // fallback is the actionable path either way.
  }

  // Path B: read bytes from disk and compile. Used by Node ESM,
  // Astro dev, Vite SSR. Cloudflare Workers can't reach this branch
  // because they'd already have returned via Path A; if Path A
  // somehow fails on Workers, the embedder rejects
  // `WebAssembly.compile` with a clear error rather than silently
  // doing the wrong thing.
  const wasmUrl = new URL(WASM_SPECIFIER, import.meta.url);
  if (wasmUrl.protocol === "file:") {
    const [{ readFile }, { fileURLToPath }] = await Promise.all([
      import("node:fs/promises"),
      import("node:url"),
    ]);
    const bytes = await readFile(fileURLToPath(wasmUrl));
    cachedModule = await WebAssembly.compile(bytes);
    return cachedModule;
  }

  // Path C: HTTP fetch — generic last resort for browser-like
  // runtimes that aren't Workers and aren't Node. Rare in our
  // deployment matrix.
  const response = await fetch(wasmUrl);
  cachedModule = await WebAssembly.compileStreaming(response);
  return cachedModule;
}

async function ensureInitialized(): Promise<void> {
  if (initialized) {
    return;
  }
  const bridgeModule = await loadBridgeModule();
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
  await ensureInitialized();
  return glue.evaluateDocuments(documentsJson, configJson);
}

export async function scoreIntelligence(
  documentsJson: string,
  manifestJson?: string | null,
): Promise<string> {
  await ensureInitialized();
  return glue.scoreIntelligence(documentsJson, manifestJson ?? undefined);
}

// Renders the LLM authoring prompt for the editor's site. Pure projection
// over the documents — no KV, no host filesystem.
export async function generateFactsPrompt(
  documentsJson: string,
): Promise<string> {
  await ensureInitialized();
  return glue.generateFactsPrompt(documentsJson);
}

// Validates a candidate facts.json against the site documents. Returns the
// raw JSON string of { validation, assessment } so the caller can render
// either inline or hand it to the React component verbatim.
export async function validateFactsManifest(
  manifestJson: string,
  documentsJson: string,
): Promise<string> {
  await ensureInitialized();
  return glue.validateFactsManifest(manifestJson, documentsJson);
}
