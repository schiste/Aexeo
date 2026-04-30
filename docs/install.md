# Internal Install and Release

This repository is private and intended for internal Rust binary distribution.

## Local Development

Run the CLI directly from source:

```bash
cargo run -p seogeo-cli -- check .
```

Install local git hooks after cloning:

```bash
sh scripts/install-quality-tools.sh
sh scripts/install-hooks.sh
```

The hard local quality gate requires these Rust-side tools:

- `cargo-audit`
- `cargo-deny`
- `cargo-udeps`
- the `nightly` Rust toolchain with `rust-src` and `llvm-tools-preview`

The installer script above provisions that exact set.

For a local release-style binary:

```bash
cargo build --release
```

The binary will be available at `target/release/seogeo-cli`.

## Deterministic Binary Install

Install a built binary into a stable destination directory:

```bash
sh scripts/install-seogeo.sh --from-binary target/release/seogeo-cli
```

By default the installer writes to `~/.local/bin/seogeo-cli` and runs a `--help`
smoke test after copying the binary.

Override the destination when needed:

```bash
sh scripts/install-seogeo.sh \
  --from-binary target/release/seogeo-cli \
  --dest-dir /opt/aexeo/bin
```

## Upgrade Procedure

1. Pull the target commit or release tag.
2. Rebuild with `cargo build --release`.
3. Re-run `sh scripts/install-seogeo.sh --from-binary target/release/seogeo-cli`.
4. Confirm the installed binary with `seogeo-cli --help` or `seogeo-cli rules`.

## Build Internal Artifacts

Create release artifacts for the host triple:

```bash
sh scripts/build_internal_release.sh
```

Or for a specific target (must match the host; refuses cross-compilation
unless `--allow-cross` is passed):

```bash
sh scripts/build_internal_release.sh --target darwin-arm64
sh scripts/build_internal_release.sh --target linux-x86_64
```

This produces:

- `dist/seogeo-cli-<os-arch>`
- `dist/seogeo-cli-<os-arch>.sha256`

The release workflow combines per-target `.sha256` files into a single
`SHA256SUMS.txt` asset on the GitHub Release.

## Distributing to Consumer Repos

The seogeo CLI is consumed from internal repos (e.g. `aeptus-com-cms`)
without re-building from source. Each tag push (`v*`) on this repo
triggers `.github/workflows/release-internal.yml`, which builds the
binary natively on `macos-14` (arm64) and `ubuntu-latest` (x86_64),
then publishes a real GitHub Release with these assets:

- `seogeo-cli-darwin-arm64`
- `seogeo-cli-linux-x86_64`
- `SHA256SUMS.txt`

Consumer repos don't need Rust, the Aexeo source, or any toolchain —
they vendor a single bootstrap script that fetches the matching
prebuilt binary at the version they pin.

### Adopting in a new consumer repo

1. Copy `scripts/bootstrap-seogeo.template.sh` from this repo into the
   consumer as `scripts/bootstrap-seogeo.sh` and `chmod +x` it.
2. Create a `.seogeo-version` file at the consumer repo root with a
   constraint, e.g. `^0.7` (caret pin) or `v0.7.4` (exact).
3. Set `GITHUB_TOKEN` to a token with `contents:read` on this repo
   (see *Auth setup* below) and run the bootstrap once locally:

   ```bash
   GITHUB_TOKEN=<token> ./scripts/bootstrap-seogeo.sh
   ```

   This resolves the constraint, writes `.seogeo-version.lock`, downloads
   the binary into `~/.cache/seogeo/<tag>/seogeo-cli`, verifies the
   SHA256, and prints the binary path on stdout.
4. Commit both `.seogeo-version` and `.seogeo-version.lock`.

### Adopting against an existing backlog

Running `seogeo-cli check .` for the first time on a real repo will
almost always surface a sizeable backlog of pre-existing findings —
generated artifacts, vendored content, log files, longstanding HTML
issues, etc. Failing CI on day one over inherited problems is a bad
adoption story; the practical pattern is:

1. **Configure excludes first.** Add a `seogeo.toml` (or equivalent
   config) in the consumer repo that excludes generated paths and
   anything outside the actual surface you intend to lint
   (e.g. `target/`, `dist/`, `node_modules/`, `.seogeo-reports/`,
   `*-latest.json`). Iterate locally with
   `seogeo-cli check . --format text` until the remaining backlog is
   real findings, not noise.

2. **Snapshot the current state as a baseline:**

   ```bash
   seogeo-cli baseline .
   ```

   This writes `.seogeo-baseline.json` recording every current finding.
   Commit it.

3. **Run check with `--regressions-only` in CI:**

   ```yaml
   - name: Run seogeo (regressions only)
     run: seogeo-cli check . --baseline .seogeo-baseline.json --regressions-only
   ```

   CI now fails only on findings introduced after the baseline was
   captured. The pre-existing backlog stays visible for paydown but
   doesn't block PRs.

4. **Pay down at your own pace.** Whenever a finding from the baseline
   gets fixed, refresh the baseline (`seogeo-cli baseline .`) and
   commit. The new baseline carries fewer findings; CI's bar rises
   accordingly.

This is the same pattern eslint, mypy, and stricter linters use for
brownfield adoption. Trying to clear the entire backlog before turning
the linter on is what kills adoption — incremental ratchet wins.

> See also [docs/static-site-ci.md](static-site-ci.md) for the full
> static-site CI recipe (build → generate machine surfaces →
> regression-gated check), monorepo layouts, and placeholder-route
> patterns.

### CI integration

The bootstrap is idempotent and cache-aware. A typical CI step:

```yaml
- name: Mint a seogeo fetch token
  id: app-token
  uses: actions/create-github-app-token@v1
  with:
    app-id: ${{ vars.SEOGEO_APP_ID }}
    private-key: ${{ secrets.SEOGEO_APP_PRIVATE_KEY }}
    owner: schiste
    repositories: Aexeo

- name: Install seogeo
  env:
    GITHUB_TOKEN: ${{ steps.app-token.outputs.token }}
  run: |
    SEOGEO_BIN=$(./scripts/bootstrap-seogeo.sh)
    echo "$(dirname "$SEOGEO_BIN")" >> "$GITHUB_PATH"

- name: Run seogeo check
  run: seogeo-cli check .
```

In CI (`$CI=true`) the bootstrap **refuses to write the lock**. If
`.seogeo-version.lock` is missing or no longer satisfies
`.seogeo-version`, the build fails with a message telling the operator
to re-run the bootstrap locally and commit the updated lock. This
matches `npm ci` / `cargo build --locked` semantics and is the
mechanism that keeps CI builds reproducible under floating pins.

### Bumping the pinned version

```bash
# 1. Edit the constraint (e.g. ^0.7 → ^0.8).
$EDITOR .seogeo-version

# 2. Drop the lock so the next bootstrap re-resolves.
rm .seogeo-version.lock

# 3. Re-run; this writes a new lock satisfying the new constraint.
./scripts/bootstrap-seogeo.sh

# 4. Review and commit both files.
git diff .seogeo-version .seogeo-version.lock
git add .seogeo-version .seogeo-version.lock
```

### Constraint syntax

| Constraint | Matches | Notes |
|------------|---------|-------|
| `v0.7.4` | exactly `v0.7.4` | leading `v` optional |
| `^0.7` or `^0.7.0` | `0.7.*` | minor pin while major is 0 |
| `^0.7.4` | `0.7.x` where `x >= 4` | patch-floor on 0.x |
| `^1.2` | `>=1.2.0, <2.0.0` | minor and patch float once major hits 1 |

Tilde (`~`) and explicit ranges (`>=`, `<`) are explicitly rejected with
a "use caret instead" error. Add them later only if a real use case
appears.

### Auth setup

Consumers use a GitHub App installation token (preferred over a long-
lived PAT) to fetch private release assets. One-time setup:

1. Create a GitHub App on the `schiste` account (`Settings → Developer
   settings → GitHub Apps → New`):
   - **Permissions** → Repository → **Contents: Read-only**
   - **Where can this be installed:** Only this account
   - Webhook: disabled (uncheck Active)
2. Generate a private key on the App page; save the `.pem` securely.
3. Note the numeric **App ID**.
4. Install the App on `schiste/Aexeo`. Do **not** install it on the
   consumer repos. The consumer side only needs the App's credentials
   (App ID + private key) — `actions/create-github-app-token` mints a
   `schiste/Aexeo`-scoped token by passing `owner: schiste,
   repositories: Aexeo` regardless of where the workflow runs. Keeping
   the App installed only on `schiste` minimizes blast radius if the
   private key ever leaks.
5. For each consumer repo:
   - In the consumer repo settings: add variable `SEOGEO_APP_ID` and
     secret `SEOGEO_APP_PRIVATE_KEY` (paste the `.pem` contents).

The CI snippet above uses `actions/create-github-app-token@v1` to mint a
short-lived installation token from those values on every run. The
`owner: schiste` line in the snippet is required when the consumer repo
lives outside `schiste/`, because the action defaults to looking up an
installation matching the workflow's owner. No PAT to rotate, no
per-person ownership, and revocation is per-installation.

For local development outside CI, a fine-grained PAT with
`contents:read` on `schiste/Aexeo` is acceptable — set it as
`GITHUB_TOKEN` in your shell.

## Release Flow

Use [docs/release.md](release.md) as the canonical release checklist.

The minimum repo-quality gate is:

```bash
sh scripts/check-repo.sh
```

That command writes per-step timing telemetry to:

```bash
.seogeo-reports/quality-timings-latest.json
```

The local CI superset is:

```bash
sh scripts/ci-local.sh
```

## Browser Crawl Notes

Browser-backed crawl is now supported locally when the repository Node dependency is installed.

- `http` is the stable native runtime crawl path and works without Node
- `auto` prefers `playwright` when a local Playwright runtime is available, otherwise it falls back to `http`
- `playwright` now uses a long-lived local browser session during a crawl instead of relaunching Chromium for every page fetch
- install the browser runtime once from the repository root:

```bash
npm install
```

- use `SEOGEO_PLAYWRIGHT_EXECUTABLE=/absolute/path/to/runner` only when you need to override the default local runner discovery
- browser-only artifacts such as traces, screenshots, console logs, and network logs still depend on the corresponding crawl capture flags

## Benchmarks

The repository ships release-mode benchmark fixtures for the static and runtime audit paths:

```bash
sh scripts/bench.sh
```

This exercises:

- a generated static site fixture for native static audits
- a local HTTP fixture server for runtime HTTP audits

To enforce the configured benchmark budgets locally:

```bash
sh scripts/check-performance.sh
```

This writes `.seogeo-reports/benchmarks-latest.json` and fails if the measured averages exceed `performance-budget.json`.

## Runtime Operations

Check browser-runtime readiness:

```bash
cargo run -p seogeo-cli -- doctor runtime --format text
```

Render a saved audit artifact into Markdown:

```bash
cargo run -p seogeo-cli -- report render .seogeo-reports/crawl-latest.json --format md
```

For long live crawls, `crawl-latest.json` is refreshed during checkpoint flushes. That means an interrupted large-site audit can still be rendered as a partial report instead of leaving only a stale previous artifact.
