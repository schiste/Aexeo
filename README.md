# Aexeo / seogeo

`seogeo` is a Rust-first SEO and GEO linting/runtime toolkit for websites.

The repository currently contains:

- `crates/seogeo-contracts`: stable finding and audit contracts
- `crates/seogeo-core`: config, rule engine, reporting, generation, and intelligence logic
- `crates/seogeo-cli`: the canonical CLI surface
- `crates/aexeo-emdash-bridge`: the WASM bridge used by the emdash plugin
- `packages/aexeo-emdash`: the published `@aeptus/aexeo-emdash` npm package
- `packages/seogeo-crawl-worker`: optional Cloudflare worker for sandboxed plugin deployments

The source tree is licensed under [MIT](LICENSE). The Rust crates are not yet published on `crates.io`; build from source or consume GitHub release artifacts. The emdash plugin ships on npm and now rebuilds its bridge WASM from the current Rust source during `npm run build`.

## Quick Start

Build and run the CLI from source:

```bash
cargo run -p seogeo-cli -- check .
```

Run the Rust test suite:

```bash
cargo test --workspace
```

Build the npm plugin:

```bash
cd packages/aexeo-emdash
npm install
npm run build
```

Install the repository hooks once per clone:

```bash
sh scripts/install-quality-tools.sh
sh scripts/install-hooks.sh
```

For a full local validation pass before opening a PR:

```bash
sh scripts/ci-local.sh
```

## Commands

```bash
cargo run -p seogeo-cli -- check .
cargo run -p seogeo-cli -- crawl http://localhost:8000 --engine http
cargo run -p seogeo-cli -- fix .
cargo run -p seogeo-cli -- generate llms .
cargo run -p seogeo-cli -- generate robots .
cargo run -p seogeo-cli -- generate sitemap . --site-url https://example.com
cargo run -p seogeo-cli -- generate links .
cargo run -p seogeo-cli -- config print . --format toml
cargo run -p seogeo-cli -- baseline .
cargo run -p seogeo-cli -- verify https://staging.example.com --baseline .seogeo-baseline.json
cargo run -p seogeo-cli -- diff baseline.json current.json
cargo run -p seogeo-cli -- docs generate .
cargo run -p seogeo-cli -- docs check .
cargo run -p seogeo-cli -- quality .
cargo run -p seogeo-cli -- snippet inspect --path . --route about
cargo run -p seogeo-cli -- indexnow validate https://example.com abc123 --path .
cargo run -p seogeo-cli -- bing-ai import bing-ai.csv --audit .seogeo-reports/crawl-latest.json
cargo run -p seogeo-cli -- search-console export .seogeo-reports/check-latest.json --site-url https://example.com --format csv
cargo run -p seogeo-cli -- publish-hook run . --changed-url https://example.com/ --indexnow-key abc123
cargo run -p seogeo-cli -- rules
cargo run -p seogeo-cli -- adapters
```

## Product Areas

- static linting for SEO/GEO structure and machine-readable artifacts
- runtime crawl with native HTTP orchestration and optional Playwright-backed browser execution
- deterministic artifact generation and safe autofix flows
- adapter and plugin architecture for framework-specific integrations
- baseline, diff, and post-deploy verification workflows
- code-generated reference docs with drift enforcement
- higher-level intelligence passes for grounding, evidence, truth, and answer-surface coverage

## Repository Docs

- [CONSTITUTION.md](CONSTITUTION.md): product framing
- [CONTRACTS.md](CONTRACTS.md): stable vs provisional integration surface
- [SPEC.md](SPEC.md): stable contract and parity target
- [docs/ENGINEERING.md](docs/ENGINEERING.md): engineering standards
- [docs/architecture.md](docs/architecture.md): runtime, core, and integration boundaries
- [docs/decisions.md](docs/decisions.md): architecture decisions
- [docs/install.md](docs/install.md): install and bootstrap paths
- [docs/release.md](docs/release.md): release checklist
- [docs/local-quality.md](docs/local-quality.md): local quality workflow
- [docs/static-site-ci.md](docs/static-site-ci.md): static-site CI recipe
- [docs/astro-ci.md](docs/astro-ci.md): Astro CI and deployment-gate workflow
- [docs/integrations.md](docs/integrations.md): snippet, Bing AI, Search Console, and IndexNow workflows
- [docs/cli.md](docs/cli.md): generated CLI reference
- [docs/config.md](docs/config.md): generated config reference
- [docs/config.schema.json](docs/config.schema.json): generated JSON Schema for config validation
- [docs/rules.md](docs/rules.md): generated rule inventory
- [docs/adapters.md](docs/adapters.md): generated adapter and plugin reference

## Notes

`cargo run -p seogeo-cli -- docs generate .` refreshes generated docs from the Rust codebase. `cargo run -p seogeo-cli -- docs check .` fails when those docs are stale.
