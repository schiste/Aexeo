# Architecture

`seogeo` is now Rust-first and split into explicit product layers.

The high-level product choices that govern this layout are recorded in [decisions.md](decisions.md).

## Contracts

Stable finding and audit contracts live in [`crates/seogeo-contracts`](../crates/seogeo-contracts).

Responsibilities:
- own stable finding serialization
- carry optional crawl site snapshots for artifact-first intelligence workflows
- provide deterministic finding fingerprints for diff and baseline workflows
- remain independent from CLI parsing, CMS integration, and runtime crawl details

Runtime crawl artifacts are the canonical handoff between collection and
intelligence. Downstream commands should prefer `--from-crawl-artifact` when
operating on live sites so they analyze the exact crawled HTML, crawl status,
machine artifacts, and route inventory rather than whatever files happen to
exist in the current working directory.

## Core

The canonical core lives in [`crates/seogeo-core`](../crates/seogeo-core).

Responsibilities:
- configuration contract and defaults
- built-in rule and adapter inventory metadata
- report rendering and audit artifact persistence
- generated documentation metadata
- baseline and regression diff primitives
- runtime performance budgets and performance diff primitives

This layer should stay independent from CLI argument parsing and website-specific runtime glue.

## Runtime Performance Observability

Runtime crawl artifacts carry both wall-clock and cumulative timing data. Wall
clock is used to understand end-user latency for a run; cumulative timings are
used to attribute crawler cost across repeated phases such as fetches, queue
planning, rule evaluation, snapshotting, checkpointing, and artifact writes.

`crawl` and `profile runtime` can evaluate a JSON performance budget with
`--perf-budget`. `perf diff` compares two runtime audit artifacts with a
configurable relative and absolute threshold, making it suitable for CI gates
where small network jitter should not be treated as a regression.

## Machine-Readable Presence

Aexeo treats AI discoverability as a graph of machine-readable surfaces rather
than as one-off files.

Core owns:

- `MachineSurfaceGraph` discovery
- deterministic `facts.json`, `llms.txt`, `llms-full.txt`, and `.md.txt`
  artifact generation
- surface-readiness audit rules
- answer fan-out coverage analysis
- runtime performance bottleneck summaries
- IndexNow validation, dry-run planning, submission ledgering, and retries

The graph records discovery source explicitly. A Markdown mirror found through a
static link is different from one found through `llms.txt`, sitemap, local
artifact loading, rendered UI, or convention probing. This distinction keeps
scores portable across websites while still allowing emerging conventions such
as Google-style `.md.txt` mirrors.

## CLI

The canonical command surface lives in [`crates/seogeo-cli`](../crates/seogeo-cli).

Responsibilities:
- own the public command contract
- generate CLI reference documentation
- expose native Rust workflows for checks, crawl, fix, generate, baseline, verify, docs, rules, adapters, trends, and diff surfaces

## Planned Monorepo Landing Shape

When Aexeo moves into the future `website` monorepo, it should move as bounded
packages, not as website application internals.

Target package map:

- `aexeo-contracts`
- `aexeo-core`
- `aexeo-cli`
- `aexeo-payload-astro-bridge`

The detailed ownership rules are documented in
[package-boundaries.md](package-boundaries.md).

## Plugin Compatibility

The repository no longer carries the legacy Python runtime implementation.

Boundary rules:
- the Rust CLI is the only supported command surface
- contracts, docs, reporting, runtime audit, and verification behavior are owned by Rust
- plugin manifest validation may still accept Python-style modules as an external compatibility surface
- browser or framework-specific tooling may still be invoked externally when that is materially more practical
