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
- staged-file placeholder and debug marker rejection in Rust and shell
- staged-file `unsafe` marker rejection
- repo-wide `cargo fmt --check`
- strict non-test Clippy policy from `scripts/clippy-strict.sh`
- repo-wide `cargo clippy --workspace --all-targets -- -D warnings`
- repo-wide `cargo test --workspace --all-targets`
- generated docs drift detection
- repo-quality policy enforcement via `seogeo quality .`
- canonical config rendering through `seogeo config print`
- example config validation through `docs/examples/seogeo.v1.toml`

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

## Why The Gate Is Strict

This repository is small enough that broad repo-wide Rust validation is still practical locally, and the cost of letting low-signal code through is higher than the cost of a slower commit.

The local system is specifically aggressive against:

- `unwrap` and `expect` in non-test Rust code
- `todo!`, `unimplemented!`, `dbg!`, and shell placeholder markers
- `unsafe` in production Rust code
- undocumented drift between code and generated docs
- missing hook scripts or missing local quality tooling
- credential-like tokens added to the staged diff
- config-surface drift that breaks the canonical versioned TOML contract

## Commands

Install hooks once per clone:

```bash
sh scripts/install-hooks.sh
```

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

## CI Alignment

GitHub Actions should reuse the same shell entrypoints rather than duplicating raw Cargo commands. The goal is one quality model with different execution environments, not separate local and remote rule sets.
