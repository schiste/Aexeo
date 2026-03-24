# Aexeo / seogeo

`seogeo` is an internal SEO and GEO linting runtime for websites.

It is being built as developer infrastructure for private use: think Ruff for search quality, retrieval structure, AI-facing artifacts, deterministic cleanup, and runtime website audits.

## Internal Use

This repository is private and intended for internal use only.

- no public package publishing
- no public release channel
- install from the private repository or from internal build artifacts

See [docs/install.md](docs/install.md) for supported installation and release paths.

## Rust-First Architecture

The Rust workspace is now the canonical entrypoint for Aexeo.

- `crates/seogeo-contracts`: stable finding and audit contracts
- `crates/seogeo-core`: config, rule inventory, reporting, docs, and diff/baseline primitives
- `crates/seogeo-cli`: canonical CLI surface

The CLI surface is fully native Rust. The Python tree remains in the repository only as historical reference material and for parity comparison while the new workspace settles.

## Commands

```bash
cargo run -p seogeo-cli -- check .
cargo run -p seogeo-cli -- crawl http://localhost:8000 --engine auto
cargo run -p seogeo-cli -- fix .
cargo run -p seogeo-cli -- generate llms .
cargo run -p seogeo-cli -- generate robots .
cargo run -p seogeo-cli -- generate links .
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
- [SPEC.md](SPEC.md): stable contract and parity target
- [docs/ENGINEERING.md](docs/ENGINEERING.md): engineering standards
- [docs/architecture.md](docs/architecture.md): runtime, core, and integration boundaries
- [docs/decisions.md](docs/decisions.md): enforced architecture decisions
- [docs/install.md](docs/install.md): internal install and release instructions
- [docs/cli.md](docs/cli.md): generated CLI reference
- [docs/config.md](docs/config.md): generated config reference
- [docs/rules.md](docs/rules.md): generated rule inventory
- [docs/adapters.md](docs/adapters.md): generated adapter and plugin reference

## Notes

`cargo run -p seogeo-cli -- docs generate .` refreshes the generated reference docs from the Rust codebase. `cargo run -p seogeo-cli -- docs check .` fails when those docs are stale.
