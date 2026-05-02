# Astro CI

`Aexeo` can act as a hard CI gate for Astro static sites and as a post-deploy verification step for preview or staging environments.

## Recommended Config

Use the canonical versioned surface and point the site adapter at Astro output:

```toml
version = 1

[site]
url = "https://example.com"
source_dir = "dist"
adapter = "astro-dist"
```

Validate the resolved config locally:

```bash
cargo run -q -p aexeo-cli -- config print . --format toml
```

## Static Build Gate

Run these commands in CI:

```bash
pnpm install --frozen-lockfile
pnpm astro build
cargo run -q -p aexeo-cli -- check . --format text
```

This is the primary hard gate. It validates the built static output, not source templates.

## Regression Gate

Use baselines when you want to block only newly introduced findings:

```bash
cargo run -q -p aexeo-cli -- baseline .
cargo run -q -p aexeo-cli -- check . --baseline .aexeo-baseline.json --regressions-only
```

This is useful for gradual adoption on large existing sites.

## Post-Deploy Verification

Use runtime verification against a deployed preview or staging URL:

```bash
cargo run -q -p aexeo-cli -- verify https://preview.example.com --baseline .aexeo-baseline.json --engine http --max-pages 500
```

If browser-backed verification becomes available in your environment, add it as a distinct supported engine rather than assuming `auto` will change behavior.

## GitHub Actions Example

See [docs/examples/astro-aexeo-ci.yml](docs/examples/astro-aexeo-ci.yml) for a copyable workflow.

## Operational Notes

- Treat `check` as the required blocking gate for pull requests.
- Treat `baseline` and `regressions-only` as an adoption bridge, not the permanent default.
- Treat `verify` as a deployment or preview-environment control.
- Keep `docs/config.schema.json` available to editors and CI so invalid config fails before builds start.
