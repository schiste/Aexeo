# Architecture Decisions

This document records the production-shape decisions for Aexeo.

## 1. Rust Is Canonical

The supported command surface is the Rust workspace.

- `crates/seogeo-contracts` owns stable contracts
- `crates/seogeo-core` owns product behavior
- `crates/seogeo-cli` owns the user-facing CLI

The legacy Python runtime has been removed from the repository and must not be reintroduced as a second implementation surface.

## 2. Plugin Compatibility May Accept Python Modules

Python remains relevant only as an external plugin manifest format accepted by `plugin-check`.

- it is a compatibility input, not an in-repo runtime
- it must not become the primary execution model again
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

## 7. A Move Into The Website Monorepo Must Preserve Package Isolation

If Aexeo moves into the future `website` repo, it must move as a package set,
not as blended application code.

- `aexeo-contracts`, `aexeo-core`, and `aexeo-cli` remain explicit packages
- website integration belongs in a separate bridge package
- the standalone CLI remains a first-class product surface
- website-specific schemas and adapters must not leak into the core engine

## 8. Config Compatibility Must Be Explicitly Versioned

The supported config contract is the nested `version = 1` TOML surface.

- legacy flat keys remain compatibility inputs, not the forward-looking contract
- compatibility mode must emit machine-readable deprecation warnings
- CI should validate config through `seogeo config print --format json`
- future config evolution should happen through versioned schema changes, not silent drift
