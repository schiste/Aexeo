# aexeo-emdash-bridge status

`aexeo-emdash-bridge` is the Rust crate that powers the WASM evaluator shipped by `@aeptus/aexeo-emdash`.

Current status:

- the crate is part of the main Cargo workspace
- it compiles and tests from the repository root
- `packages/aexeo-emdash/scripts/build-wasm.sh` rebuilds the published bridge assets from this source tree
- `aexeo-core` exposes a `net` feature so the bridge can depend on the core without pulling network-only code into the WASM target

Useful commands:

```bash
cargo test -p aexeo-emdash-bridge
npm --prefix packages/aexeo-emdash run build:wasm
```
