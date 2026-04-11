# Engineering Standards

This document defines implementation standards for `seogeo`.

## Goals

The codebase should be:
- highly testable
- highly documented
- explicitly abstracted at module boundaries
- stable as a Rust-first product without changing product semantics

## Function Design

Every function should have one distinct responsibility.

That is what "every function must be unique" means in this project:
- no duplicated logic under different names
- no helper that partially overlaps another helper without a strong reason
- no large mixed-responsibility function when it can be split into named units
- no separate source of truth for the same behavior in multiple modules

Examples already enforced in the codebase:
- route/page selection lives in `site.rs`
- rule registration lives in `registry.rs`
- config parsing lives in `config/`
- output shape lives in `contracts`

## Documentation Standard

At minimum, every public module and every non-trivial public function should have a docstring.

Required documentation layers:
- product contract: [SPEC.md](../SPEC.md)
- product framing: [CONSTITUTION.md](../CONSTITUTION.md)
- system architecture: [architecture.md](architecture.md)
- architecture decisions: [decisions.md](decisions.md)
- rule inventory: [rules.md](rules.md)
- engineering expectations: this document

## Testing Standard

New logic should ship with tests in the same change.

Expected coverage style:
- unit tests for pure helpers
- parser/model tests for inventory logic
- rule tests for each rule family
- smoke tests for CLI behavior
- binary-level integration tests for the supported Rust CLI surface

The preferred pattern is deterministic fixture construction with small local HTML strings rather than large snapshots.

## Local Enforcement

This repository treats `pre-commit` as the hardest local quality gate.

That means:
- staged-file protections should stay fast and high-signal
- repo-wide Rust validation is allowed at commit time
- `unwrap`, `expect`, `todo!`, `unimplemented!`, `dbg!`, and `unsafe` in non-test Rust code are policy failures
- release and install smoke belong in `pre-push` or `ci-local`, not hidden ad hoc scripts

The canonical local workflow is documented in [local-quality.md](local-quality.md).

## Abstraction Standard

Abstractions should exist where they eliminate duplicated semantics, not where they only rename complexity.

Good abstractions:
- route normalization helpers
- rule registry
- shared test fixtures
- explicit page parsing functions

Bad abstractions:
- wrappers that hide one line of obvious code
- generic factories with no second use case
- inheritance for rule modules that do not share meaningful behavior

## Portability

When adding behavior, ask:
1. Is this product behavior or implementation detail?
2. If we refactor the Rust implementation, what must stay identical?

If the answer is user-visible semantics, it belongs in [SPEC.md](../SPEC.md).
