# Local Quality Workflow

This repository treats local quality enforcement as a product contract, not a convenience lint layer.

`pre-commit` is the hardest gate on purpose. A change should be expensive to commit if it lowers code quality, weakens type and safety guarantees, or lets AI-generated slop through.

## Layers

### `pre-commit`

Installed through `sh scripts/install-hooks.sh` and executed from `.githooks/pre-commit`.

This is the hardest gate. It runs:

```bash
sh scripts/guard-staged.sh
sh scripts/check-repo.sh
```

What it enforces:

- staged diff sanity with `git diff --cached --check`
- staged secret and credential leakage detection
- staged diff rejection for newly added `TODO` and `FIXME` markers in Rust and shell files
- repo-wide `cargo fmt --check`
- strict non-test Clippy policy from `scripts/clippy-strict.sh`
- repo-wide `cargo clippy --workspace --all-targets -- -D warnings`
- repo-wide `cargo test --workspace --all-targets`
- generated docs drift detection
- repo-quality policy enforcement via `seogeo quality .`
- canonical config rendering through `seogeo config print`
- example config validation through `docs/examples/seogeo.v1.toml`

This run also writes per-step timing telemetry to:

```bash
.seogeo-reports/quality-timings-latest.json
```

Each step records:

- command name
- started and finished timestamps
- duration in milliseconds
- exit code
- cache hint describing whether the gate is likely cache-light, cache-sensitive, or network-sensitive

This layer is intentionally heavy because it is the main quality firewall for this repository.

### `pre-push`

Executed from `.githooks/pre-push`.

This layer is narrower than `pre-commit` and focuses on install and packaging realism:

```bash
sh scripts/pre-push.sh
```

What it enforces:

- `cargo build --release`
- install-path smoke test through `scripts/install-seogeo.sh`

### Local CI

Run manually before opening a PR:

```bash
sh scripts/ci-local.sh
```

Optional dependency audit:

```bash
sh scripts/ci-local.sh --with-audit
```

`ci-local.sh` runs the full pre-commit gate, then the pre-push gate, and optionally `cargo audit` when available.

It now also enforces release-mode benchmark budgets through:

```bash
sh scripts/check-performance.sh
```

This compares the static and runtime benchmark fixtures against `performance-budget.json` and writes `.seogeo-reports/benchmarks-latest.json`.

`ci-local.sh` also refreshes `.seogeo-reports/quality-timings-latest.json` with top-level timing data for:

- `check-repo`
- `pre-push`
- `check-performance`

## Why The Gate Is Strict

This repository is small enough that broad repo-wide Rust validation is still practical locally, and the cost of letting low-signal code through is higher than the cost of a slower commit.

The local system is specifically aggressive against:

- newly added `TODO` and `FIXME` markers in staged Rust and shell diffs
- `unwrap` and `expect` in non-test Rust code
- `todo!`, `unimplemented!`, and `dbg!` in non-test Rust code
- `unsafe` in production Rust code
- undocumented drift between code and generated docs
- missing hook scripts or missing local quality tooling
- credential-like tokens added to the staged diff
- config-surface drift that breaks the canonical versioned TOML contract

## Commands

Install hooks once per clone:

```bash
sh scripts/install-quality-tools.sh
sh scripts/install-hooks.sh
```

The hard local gate depends on:

- `cargo-audit`
- `cargo-deny`
- `cargo-udeps`
- `nightly` plus `rust-src` and `llvm-tools-preview`

Use `sh scripts/install-quality-tools.sh` to provision them.

Run the hardest local gate explicitly:

```bash
sh scripts/pre-commit.sh
```

Run release/install validation:

```bash
sh scripts/pre-push.sh
```

Run the full local pipeline:

```bash
sh scripts/ci-local.sh
```

Inspect browser-runtime readiness explicitly:

```bash
cargo run -p seogeo-cli -- doctor runtime --format text
```

Render a saved audit artifact into Markdown or text:

```bash
cargo run -p seogeo-cli -- report render .seogeo-reports/crawl-latest.json --format md
```

For large runtime audits, the latest crawl artifact is now refreshed during checkpoint intervals. If a live crawl is interrupted or still in progress, `crawl-latest.json` can still be rendered as a partial audit with current crawl stats.

## CI Alignment

GitHub Actions should reuse the same shell entrypoints rather than duplicating raw Cargo commands. The goal is one quality model with different execution environments, not separate local and remote rule sets.
