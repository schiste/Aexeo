// Bundle src/sandbox-entry.ts into a single self-contained ESM file.
//
// Why this exists: emdash's Cloudflare sandbox runner reads the plugin
// entrypoint with readFileSync and hands the resulting string to Worker
// Loader as a single module ("sandbox-plugin.js"). Relative imports
// between files in the plugin's dist/ are not resolved at runtime — the
// V8 isolate sees only the modules the host registers, so any
// `./mcp.js` import that survives into the bundled output triggers
// "No such module" at startup.
//
// We use esbuild because it ships an ES-module bundler with native
// support for inlining via the `binary` loader, which is what we need
// for the wasm-bindgen "bundler"-target glue that imports the raw .wasm
// file as if it were a module.

import { readFile, writeFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");

// Substitute the WASM-backed evaluator with a sandbox-safe stub when
// building the sandbox bundle. The WASM evaluator's module-level
// WebAssembly.instantiate exceeds Worker Loader's default 50ms cpuMs
// limit, leaving top-level await unsettled at load. Heavy evaluation
// runs out-of-sandbox; the sandbox reads pre-computed findings from KV.
// The astro integration's CI-gate path keeps using ./evaluator.ts.
const sandboxEvaluatorPlugin = {
  name: "sandbox-evaluator",
  setup(buildApi) {
    buildApi.onResolve({ filter: /(^|\/)evaluator(\.js)?$/ }, (args) => {
      // Only redirect imports from inside our own src/ tree, not
      // anything that happens to end in "evaluator". Belt and braces
      // — there's only one such importer today (plugin.ts).
      if (!args.importer.includes("/emdash-plugin-seogeo/src/")) {
        return null;
      }
      return {
        path: resolve(root, "src/evaluator-sandbox.ts"),
      };
    });
  },
};

// Custom esbuild plugin that resolves the .wasm import emitted by
// wasm-bindgen's bundler-target glue (`import * as wasm from
// "./aexeo_emdash_bridge_bg.wasm"`). We base64-encode the bytes and
// generate a synthetic JS module that instantiates the module at
// load time and re-exports the instance's exports. The synthetic
// module uses top-level await; the resulting ESM bundle is async,
// which the sandbox runner handles transparently.
//
// NOTE: This plugin is no longer reached when the sandbox-evaluator
// substitution above is in effect, since the stub doesn't import the
// WASM glue. We keep it in the build for symmetry — if a future
// non-sandbox bundle target is added, the same machinery will work.
const wasmInlinePlugin = {
  name: "wasm-inline",
  setup(buildApi) {
    buildApi.onResolve({ filter: /\.wasm$/ }, (args) => {
      return {
        path: resolve(args.resolveDir, args.path),
        namespace: "wasm-inline",
      };
    });

    buildApi.onLoad(
      { filter: /.*/, namespace: "wasm-inline" },
      async (args) => {
        const bytes = await readFile(args.path);
        const base64 = bytes.toString("base64");
        // Resolve the matching wasm-bindgen JS glue file in the same
        // directory. Its named exports (__wbg_*, __wbindgen_*) are the
        // imports the WASM module needs to link against.
        const glueRelPath = "./aexeo_emdash_bridge_bg.js";
        const contents = `
import * as glue from ${JSON.stringify(glueRelPath)};

const base64 = ${JSON.stringify(base64)};
const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
const { instance } = await WebAssembly.instantiate(bytes, {
  ${JSON.stringify(glueRelPath)}: glue,
});
export default instance.exports;
export const memory = instance.exports.memory;
`;
        return {
          contents,
          loader: "js",
          resolveDir: dirname(args.path),
        };
      },
    );
  },
};

// wasm-bindgen's entry glue does `import * as wasm from "./...wasm"`
// and treats the namespace object as the exports table. With our
// synthetic module above, the namespace has `default` (the exports
// table) plus named re-exports (memory). That's compatible with how
// the glue uses `wasm.memory` / `wasm.evaluateDocuments` etc.: with
// `* as wasm` the namespace exposes both. We re-export each function
// by name so destructured access keeps working — see the patch below.
const wasmGlueShimPlugin = {
  name: "wasm-glue-shim",
  setup(buildApi) {
    buildApi.onLoad(
      { filter: /aexeo_emdash_bridge\.js$/ },
      async (args) => {
        // Replace the original entry glue with one that pulls every
        // export off the instantiated module via a single namespace
        // import. This sidesteps the wasm-bindgen assumption that
        // `import * as wasm` yields the WebAssembly.Instance.exports
        // table directly — our synthetic wasm module exports the
        // table as `default`, so we forward it explicitly.
        const contents = `
import * as wasmNs from "./aexeo_emdash_bridge_bg.wasm";
import { __wbg_set_wasm } from "./aexeo_emdash_bridge_bg.js";
const wasm = wasmNs.default;
__wbg_set_wasm(wasm);
wasm.__wbindgen_start();
export { evaluateDocuments, scoreIntelligence } from "./aexeo_emdash_bridge_bg.js";
`;
        return { contents, loader: "js", resolveDir: dirname(args.path) };
      },
    );
  },
};

await mkdir(resolve(root, "dist"), { recursive: true });

const result = await build({
  entryPoints: [resolve(root, "src/sandbox-entry.ts")],
  outfile: resolve(root, "dist/sandbox-entry.js"),
  bundle: true,
  format: "esm",
  platform: "neutral",
  target: "es2022",
  // Workers / Worker Loader support TLA in module scripts.
  supported: { "top-level-await": true },
  // Don't mangle — we want readable output for stack traces.
  minify: false,
  sourcemap: false,
  legalComments: "none",
  plugins: [sandboxEvaluatorPlugin, wasmGlueShimPlugin, wasmInlinePlugin],
  logLevel: "info",
});

if (result.errors.length > 0) {
  process.exit(1);
}

const outPath = resolve(root, "dist/sandbox-entry.js");
const stat = await readFile(outPath);
console.log(`bundled: dist/sandbox-entry.js (${stat.length.toLocaleString()} bytes)`);
