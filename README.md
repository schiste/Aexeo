# Aexeo / seogeo

`seogeo` is an internal SEO and GEO linting runtime for websites.

It is being built as developer infrastructure for private use: think Ruff for search quality, retrieval structure, AI-facing artifacts, deterministic cleanup, and runtime website audits.

## Internal Use

This repository is private and intended for internal use only.

- no public package publishing
- no public release channel
- install from the private repository or from internal build artifacts
- use `sh scripts/install-seogeo.sh --from-binary target/release/seogeo-cli` for deterministic local installs

See [docs/install.md](docs/install.md) for supported installation and release paths.

## Local Quality

Install the repository hooks once per clone:

```bash
sh scripts/install-quality-tools.sh
sh scripts/install-hooks.sh
```

`pre-commit` is intentionally the hardest local gate in this repository. It runs staged-file safeguards plus the full repo quality sequence from `scripts/check-repo.sh`.

For a full local validation pass before opening a PR:

```bash
sh scripts/ci-local.sh
```

## Rust-First Architecture

The Rust workspace is now the canonical entrypoint for Aexeo.

- `crates/seogeo-contracts`: stable finding and audit contracts
- `crates/seogeo-core`: config, rule inventory, reporting, docs, and diff/baseline primitives
- `crates/seogeo-cli`: canonical CLI surface

The CLI surface is fully native Rust. The legacy Python implementation has been removed from the repository; only plugin manifest validation still accepts Python-style plugin modules as an integration input.

## Commands

```bash
cargo run -p seogeo-cli -- check .
cargo run -p seogeo-cli -- crawl http://localhost:8000 --engine http
cargo run -p seogeo-cli -- fix .
cargo run -p seogeo-cli -- generate llms .
cargo run -p seogeo-cli -- generate robots .
cargo run -p seogeo-cli -- generate links .
cargo run -p seogeo-cli -- config print . --format toml
cargo run -p seogeo-cli -- baseline .
cargo run -p seogeo-cli -- verify https://staging.example.com --baseline .seogeo-baseline.json
cargo run -p seogeo-cli -- diff baseline.json current.json
cargo run -p seogeo-cli -- docs generate .
cargo run -p seogeo-cli -- docs check .
cargo run -p seogeo-cli -- quality .
cargo run -p seogeo-cli -- rules
cargo run -p seogeo-cli -- adapters
```

## Current Product Areas

- static linting for SEO/GEO structure and artifacts
- runtime crawl with native HTTP orchestration and room for external browser-backed execution
- deterministic artifact generation and safe HTML/artifact autofix
- adapter and plugin architecture for framework-specific usage
- baseline, diff, and post-deploy verification workflows
- code-generated reference docs with drift enforcement

## Repository Docs

- [CONSTITUTION.md](CONSTITUTION.md): product framing
- [CONTRACTS.md](CONTRACTS.md): stable vs provisional integration surface
- [SPEC.md](SPEC.md): stable contract and parity target
- [docs/ENGINEERING.md](docs/ENGINEERING.md): engineering standards
- [docs/architecture.md](docs/architecture.md): runtime, core, and integration boundaries
- [docs/decisions.md](docs/decisions.md): enforced architecture decisions
- [docs/package-boundaries.md](docs/package-boundaries.md): target package map for the future `website` monorepo move
- [docs/install.md](docs/install.md): internal install and upgrade instructions
- [docs/local-quality.md](docs/local-quality.md): local hook model and repo-quality enforcement
- [docs/release.md](docs/release.md): internal release checklist and packaging flow
- [docs/astro-ci.md](docs/astro-ci.md): Astro CI and deployment-gate workflow
- [docs/cli.md](docs/cli.md): generated CLI reference
- [docs/config.md](docs/config.md): generated config reference
- [docs/config.schema.json](docs/config.schema.json): generated JSON Schema for config validation
- [docs/rules.md](docs/rules.md): generated rule inventory
- [docs/adapters.md](docs/adapters.md): generated adapter and plugin reference

## Notes

`cargo run -p seogeo-cli -- docs generate .` refreshes the generated reference docs from the Rust codebase. `cargo run -p seogeo-cli -- docs check .` fails when those docs are stale.
