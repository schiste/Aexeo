# Package Boundaries

This document defines the package layout Aexeo must preserve when it moves into
the future `website` monorepo.

The goal is to move operationally, not to dissolve Aexeo into website-specific
application code.

## Current Aexeo Packages

The current standalone repository already has the three core packages that must
survive the move.

- `crates/seogeo-contracts`
- `crates/seogeo-core`
- `crates/seogeo-cli`

## Target Monorepo Landing Shape

Inside the future `website` repo, the target shape should be:

```text
packages/
  aexeo-contracts/
  aexeo-core/
  aexeo-cli/
  aexeo-emdash-bridge/
```

Additional website-specific modules may depend on these packages, but they must
not replace them.

## Ownership Rules

### `aexeo-contracts`

Owns:

- stable serialized types
- rule metadata semantics
- finding fingerprints
- integration-facing request and result contracts, when added

Must not own:

- CLI parsing
- site-specific heuristics
- emdash imports
- Astro imports

### `aexeo-core`

Owns:

- config and policy handling
- site inventory and runtime audit
- built-in rules
- reporting
- diff and baseline logic
- deterministic generation and fix logic

Must not own:

- emdash schema knowledge
- Astro component knowledge
- editor UI behavior
- queue or job transport specifics

### `aexeo-cli`

Owns:

- command parsing
- binary surface
- CLI integration tests
- generated CLI reference

Must not own:

- website-specific business rules
- CMS hook logic

### `aexeo-emdash-bridge`

Owns:

- document-to-route resolution
- document-to-preview resolution
- emdash-facing recommendation services
- Astro build and preview adapters
- Portable Text to virtual site input conversion
- findings storage and retrieval helpers

Must not own:

- the canonical rule engine
- custom finding formats that diverge from contracts
- rule identifiers that bypass Aexeo core

## Anti-Coupling Rules

These rules must stay true before and after the move.

- `aexeo-core` must not import website app code.
- `aexeo-core` must not depend on emdash schemas.
- `aexeo-core` must not require Astro-specific route models.
- `aexeo-cli` must remain runnable without the CMS.
- website-specific integrations must consume the stable contract, not internal
  helper types from random core modules.

## Move Readiness Checklist

Aexeo is ready to move when:

- stable contracts are documented in [../CONTRACTS.md](../CONTRACTS.md)
- the stable product contract is documented in [../SPEC.md](../SPEC.md)
- the package boundary rules in this file are accepted
- Rust CLI integration tests remain green
- generated docs remain code-derived and drift-checked
- the move can preserve a first-class standalone CLI surface

## Non-Goals For The Move

The move is not an excuse to:

- merge Aexeo into generic website utilities
- drop the standalone CLI
- hardwire Aexeo to one emdash schema
- make the website repo the only place where audits can run
