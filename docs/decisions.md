# Architecture Decisions

This document records the production-shape decisions for Aexeo.

## 1. Rust Is Canonical

The supported command surface is the Rust workspace.

- `crates/seogeo-contracts` owns stable contracts
- `crates/seogeo-core` owns product behavior
- `crates/seogeo-cli` owns the user-facing CLI

No new product behavior should be added to the Python implementation.

## 2. Python Is Legacy Reference Only

`src/seogeo` remains in the repository only as migration reference material.

- it may be used for parity comparison
- it must not become the primary runtime again
- docs, CI, and release flow must target Rust first

## 3. CI And Releases Are Binary-Oriented

Internal validation and release flow should operate on the Rust workspace and produced binaries.

- `cargo test` is the primary correctness gate
- `cargo fmt --check` and `cargo clippy` are expected quality gates
- release artifacts should come from Rust builds, not Python packaging

## 4. Runtime Crawl Is Native HTTP First

The supported runtime crawl path is native Rust HTTP orchestration.

- HTTP crawl is the stable default
- browser-backed crawl may be layered in later
- browser integration must not reintroduce a Python CLI bridge

## 5. Plugins Are Contract-Bounded

Plugin behavior must stay bounded by Aexeo contracts.

- manifest validation may support legacy Python modules
- the core execution model remains Rust-owned
- future plugin execution should move toward explicit contracts, not implicit runtime coupling

## 6. The Repo Must Carry Binary-Level CLI Coverage

Unit tests are not enough for the public command surface.

The repository must keep Rust CLI integration tests that execute the built binary against controlled fixtures.
