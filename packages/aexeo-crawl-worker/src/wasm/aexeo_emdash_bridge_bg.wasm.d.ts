// In a modules-based Cloudflare Worker, Wrangler resolves
// `import x from "./foo.wasm"` to a precompiled WebAssembly.Module at
// bundle time. The wasm-bindgen-generated .d.ts that originally lived
// here described the wasm's export shape as if it were an ESM with
// named exports — that's the bundler/Vite contract, not the Workers
// one. Replace it with the modules-Worker-correct declaration.
//
// Hand-maintained and tracked: wasm-bindgen overwrites the ESM-shaped
// version on every `npm run build:wasm`, so tracking its output would
// reintroduce exactly the stale-binary problem this package's .gitignore
// exists to prevent. If the bridge's exported function set changes, this
// file does not need to change — it only describes the module binding.
declare const bridgeModule: WebAssembly.Module;
export default bridgeModule;
