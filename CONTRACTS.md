# Aexeo Stable Contracts

This document defines the stable external contracts that must remain intact while
Aexeo is integrated into other repositories, including the future `website`
monorepo.

It complements [SPEC.md](SPEC.md) by separating what is stable enough to
integrate now from what remains intentionally tunable.

## Stable Now

The following surfaces are stable and can be safely consumed by emdash,
an Astro build pipeline, or a repo-level CLI integration.

### Product identity

- Rust is the canonical runtime.
- The supported crates are:
  - `crates/seogeo-contracts`
  - `crates/seogeo-core`
  - `crates/seogeo-cli`

### Stable external types

- `Finding`
- `FindingFingerprint`
- `RuleClass`
- `ConfidenceLevel`
- `RuleMetadata`

These are owned by `crates/seogeo-contracts`.

### Stable rule identifiers

Rule IDs are product identifiers and must not be renamed casually.

Stable prefixes:

- `SEO`
- `LNK`
- `MAP`
- `ROB`
- `SOC`
- `SCH`
- `LLM`
- `CNT`
- `GEO`
- `CRW`
- `DEP`
- `QLT`

### Stable rule metadata semantics

- `RuleClass::Hard`
- `RuleClass::Policy`
- `RuleClass::Heuristic`

- `ConfidenceLevel::High`
- `ConfidenceLevel::Medium`
- `ConfidenceLevel::Low`

These semantics are part of the public reporting and integration contract.

### Stable command families

- `check`
- `crawl`
- `verify`
- `baseline`
- `diff`
- `trend`
- `generate`
- `fix`
- `quality`
- `docs`
- `rules`
- `adapters`
- `plugin-check`

### Stable review scopes

- document-oriented review
- page-oriented review
- site-oriented review
- runtime review

### Stable execution surfaces

- editor
- preview
- build
- ci

### Stable artifact contract

- retained audit artifacts under `.seogeo-reports/`
- `*-latest.json` as the stable latest artifact name
- timestamped retained history logs
- `*-trends.json` trend snapshots

### Stable integration boundaries

Integrators may rely on these boundary categories:

- document -> route resolution
- document -> preview target resolution
- static build output -> site input
- runtime URL -> runtime audit target

These are stable categories even if the exact integration package evolves.

## Stable Soon, But Still Flexible

These concepts are important enough to keep, but their exact behavior may still
be refined.

- `PageKind`
- deployment model classification
- crawl confidence accounting
- policy profiles
- route-pattern overrides

Integrations should use these concepts, but should not depend on current
inference details.

## Explicitly Provisional

The following are intentionally not frozen yet.

- page-kind inference heuristics
- GEO scoring thresholds
- schema recommendation heuristics
- supporting-block detection thresholds
- citation/source heuristics
- chunk-overlap heuristics
- exact report wording
- default policy tuning
- default route-pattern exceptions

These may improve between internal releases without being treated as contract
breaks, as long as the stable types and rule identities remain intact.

## Move Readiness Rule

Aexeo may move into the future `website` monorepo only as a bounded package set.

What must remain true after the move:

- `aexeo-core` stays independent from emdash and Astro schema details
- `aexeo-cli` stays first-class and testable outside the CMS
- integration code depends on Aexeo contracts instead of importing internal
  engine details
- website-specific adapters live outside the core engine

The target package map is documented in
[docs/package-boundaries.md](docs/package-boundaries.md).
