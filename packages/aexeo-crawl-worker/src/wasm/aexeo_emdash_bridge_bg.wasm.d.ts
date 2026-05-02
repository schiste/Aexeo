// In a modules-based Cloudflare Worker, Wrangler resolves
// `import x from "./foo.wasm"` to a precompiled WebAssembly.Module at
// bundle time. The wasm-bindgen-generated .d.ts that originally lived
// here described the wasm's export shape as if it were an ESM with
// named exports — that's the bundler/Vite contract, not the Workers
// one. Replace it with the modules-Worker-correct declaration.
declare const bridgeModule: WebAssembly.Module;
export default bridgeModule;
