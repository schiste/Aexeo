# Contributing

## Development Setup

Install the local quality tooling and hooks:

```bash
sh scripts/install-quality-tools.sh
sh scripts/install-hooks.sh
```

Core validation commands:

```bash
cargo test --workspace
cargo run -p seogeo-cli -- quality . --format json
cd packages/aexeo-emdash && npm install && npm run build
```

## Pull Requests

- Keep changes scoped and reviewable.
- Add or update tests when behavior changes.
- Regenerate docs when command or config output changes.
- Avoid committing generated noise unrelated to the change.

## Commit Quality

The repository expects:

- passing Rust tests
- passing package builds for modified npm packages
- no newly introduced secrets, TODO markers, or stale generated docs

If you change public-facing behavior, update the relevant README or `docs/` page in the same change.
