# Architecture

`seogeo` is now Rust-first and split into explicit product layers.

The high-level product choices that govern this layout are recorded in [decisions.md](decisions.md).

## Contracts

Stable finding and audit contracts live in [`crates/seogeo-contracts`](../crates/seogeo-contracts).

Responsibilities:
- own stable finding serialization
- provide deterministic finding fingerprints for diff and baseline workflows
- remain independent from CLI parsing, CMS integration, and runtime crawl details

## Core

The canonical core lives in [`crates/seogeo-core`](../crates/seogeo-core).

Responsibilities:
- configuration contract and defaults
- built-in rule and adapter inventory metadata
- report rendering and audit artifact persistence
- generated documentation metadata
- baseline and regression diff primitives

This layer should stay independent from CLI argument parsing and website-specific runtime glue.

## CLI

The canonical command surface lives in [`crates/seogeo-cli`](../crates/seogeo-cli).

Responsibilities:
- own the public command contract
- generate CLI reference documentation
- expose native Rust workflows for checks, crawl, fix, generate, baseline, verify, docs, rules, adapters, trends, and diff surfaces

## Legacy Python Reference

The Python implementation under [`src/seogeo`](../src/seogeo) remains in the repository only as reference material during the migration hardening period.

Boundary rules:
- the Rust CLI is the only supported command surface
- contracts, docs, reporting, runtime audit, and verification behavior are owned by Rust
- Python should not gain new platform semantics
- browser or framework-specific tooling may still be invoked externally when that is materially more practical
