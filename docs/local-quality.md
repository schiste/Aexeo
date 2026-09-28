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
- repo-quality policy enforcement via `aexeo-cli quality .`
- canonical config rendering through `aexeo-cli config print`
- example config validation through `docs/examples/aexeo.v1.toml`

This run also writes per-step timing telemetry to:

```bash
.aexeo-reports/quality-timings-latest.json
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
- install-path smoke test through `scripts/install-aexeo.sh`

### Local CI

Run manually before opening a PR:

```bash
sh scripts/ci-local.sh
```

`ci-local.sh` accepts these options:

```bash
sh scripts/ci-local.sh                 # full run
sh scripts/ci-local.sh --staged-guard  # force the secret scan even with an empty index
sh scripts/ci-local.sh --help
```

Any other argument prints `unknown argument` and exits `2`. For a smaller
scope, call the underlying scripts directly (`sh scripts/check-repo.sh`,
`sh scripts/pre-push.sh`, `sh scripts/check-performance.sh`).

`--with-audit` is accepted but does nothing: the dependency audit
(`cargo audit`, `cargo deny check`, `cargo +nightly udeps`) already runs
unconditionally as part of the `check-repo` stage, via `check-deps.sh`. The
flag exists so the older documented invocation still succeeds instead of
exiting `2`, and so the run summary states plainly that the audit ran.

`ci-local.sh` runs four stages in order: `guard-staged`, the full
`check-repo.sh` gate, the pre-push gate, and the performance budget check.
`cargo audit` is not optional in this path: `check-repo.sh` always runs
`check-deps.sh`, which hard-fails without `cargo-audit`, `cargo-deny`, and
nightly `cargo-udeps`.

`guard-staged.sh` runs automatically when `ci-local.sh` is executed inside a
git work tree with a non-empty index, and is skipped with a printed reason
otherwise (an empty index has no staged diff to scan). Use
`--staged-guard` to run it regardless. Outside a git work tree the only way
to exercise that layer is `sh scripts/guard-staged.sh` directly, or the
`.githooks/pre-commit` hook.

`guard-staged.sh` requires `ripgrep`. If `rg` is missing it now exits `1`
with an install hint instead of silently passing — an absent scanner used to
look identical to a clean staged diff. `sh scripts/install-quality-tools.sh`
installs it alongside the cargo tools.

It enforces release-mode benchmark budgets through:

```bash
sh scripts/check-performance.sh
```

This compares the static and runtime benchmark fixtures against `performance-budget.json` and writes `.aexeo-reports/benchmarks-latest.json`.

Runtime audit artifacts can also be compared directly:

```bash
aexeo-cli perf diff .aexeo-reports/crawl-baseline.json .aexeo-reports/crawl-latest.json
```

The command exits non-zero when one or more metrics regress beyond the configured relative threshold. Timing metrics can also use an absolute millisecond threshold to ignore small jitter, and the command emits warnings when the two runs are not directly comparable, such as different engines or page counts.

`ci-local.sh` also refreshes `.aexeo-reports/quality-timings-latest.json` with top-level timing data for:

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
cargo run -p aexeo-cli -- doctor runtime --format text
```

Render a saved audit artifact into Markdown or text:

```bash
cargo run -p aexeo-cli -- report render .aexeo-reports/crawl-latest.json --format md
```

For large runtime audits, the latest crawl artifact is now refreshed during checkpoint intervals. If a live crawl is interrupted or still in progress, `crawl-latest.json` can still be rendered as a partial audit with current crawl stats.

## CI Alignment

GitHub Actions should reuse the same shell entrypoints rather than duplicating raw Cargo commands. The goal is one quality model with different execution environments, not separate local and remote rule sets.
