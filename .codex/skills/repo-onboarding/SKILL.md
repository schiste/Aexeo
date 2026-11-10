---
name: repo-onboarding
description: Use when starting work in an unfamiliar repository, when the task asks for repo overview, setup, architecture, entrypoints, test commands, or where to begin. Skip for narrow file-scoped edits once the relevant paths are already known.
---

# Repo Onboarding: Aexeo

## When to Use

- Load this skill first when the repository is unfamiliar or the request is broad.
- Recommended when: first task in repo, repo overview, setup or run instructions, architecture or entrypoints, where should I start, broad debugging or feature-localization request.
- Skip when: known file-scoped edit, follow-up inside already identified area, task already localized to concrete files.
- Use `.codex/skills/aethyme/SKILL.md` or `.claude/skills/aethyme/SKILL.md` for Aethyme's short operating contract after orientation; load its `references/` files only when needed.

## Repo Identity

- Kind: `monorepo`
- Languages: `rust, typescript`
- Package manager: `cargo`
- Key manifests: `Cargo.toml, crates/aexeo-cli/Cargo.toml, crates/aexeo-contracts/Cargo.toml, crates/aexeo-core/Cargo.toml, crates/aexeo-emdash-bridge/Cargo.toml, crates/tempfile/Cargo.toml, package.json, packages/aexeo-crawl-worker/package.json, packages/aexeo-emdash/package.json, packages/aexeo-emdash/tests/fixtures/plugin-host/package.json`

## Workspaces

- `.` (primary; cargo; manifest `Cargo.toml`; high confidence)
- `.` (supporting; npm; manifest `package.json`; high confidence)
- `packages/aexeo-crawl-worker` (supporting; npm; manifest `packages/aexeo-crawl-worker/package.json`; high confidence)
- `packages/aexeo-emdash` (supporting; npm; manifest `packages/aexeo-emdash/package.json`; high confidence)
- `packages/aexeo-emdash/tests/fixtures/plugin-host` (supporting; npm; manifest `packages/aexeo-emdash/tests/fixtures/plugin-host/package.json`; high confidence)

## Start Here

- `dev`: `cargo run`
- `fast_test`: `cargo test --workspace`
- `build`: `cargo build --workspace`

## Supporting Commands

- `npm --prefix packages/aexeo-crawl-worker run dev` (dev; high confidence from `packages/aexeo-crawl-worker/package.json`)
  Workspace: `packages/aexeo-crawl-worker`
- `cargo run` (dev; medium confidence from `Cargo.toml`)
- `cargo test` (fast_test; high confidence from `Cargo.toml`)
- `cargo test --workspace` (fast_test; high confidence from `Cargo.toml`)
  Workspace: `.`
- `npm --prefix packages/aexeo-crawl-worker run test` (fast_test; high confidence from `packages/aexeo-crawl-worker/package.json`)
  Workspace: `packages/aexeo-crawl-worker`

## Repo Map

- `.github` (automation; automation and CI configuration; high confidence)
- `docs` (docs; documentation area; high confidence)
- `packages` (workspace; workspace-style package container; high confidence)
- `scripts` (tooling; developer tooling or scripts; high confidence)

## Aethyme Recipes

- `aethyme explore --repo "$PWD" --request "<task>" --format answer-json`
  Purpose: Broad repository orientation for a user request
- `aethyme repo inspect "$PWD" --mode brief --json-output`
  Purpose: Quick deterministic repo summary
- `aethyme graph callers "$PWD" "<symbol-or-file>" --json-output`
  Purpose: Trace likely impact before editing

## Generated and Dangerous Paths

- Generated/vendor `.aethyme/generated`: tracked generated or vendored surface; verify ownership before editing
- Sensitive `.aethyme/gates.toml`: repository validation policy; changes affect every broker submission
- Sensitive `.github/workflows`: repository automation; changes can affect publication or shared CI

## Maintainer Notes

- EmDash WASM integration: run `npm ci --prefix packages/aexeo-emdash`, then `npm run build --prefix packages/aexeo-emdash` before `npm run test:wasm --prefix packages/aexeo-emdash`. The smoke tests import the bundled `dist/content.js`; building TypeScript and WASM alone does not create it.
- Run `cargo fmt --all -- --check` after adding or moving a module: it also checks the parent `mod` declarations, which a focused test run does not.
- For a broker-coordinated `gh pr merge --match-head-commit`, pass the full `headRefOid` from `gh pr view --json headRefOid`; an abbreviated SHA is rejected.
- If a broker push fails with an internal remote hook error, keep the commit and read the error, then retry through the broker without force. If it persists, report the rejection and keep the session for recovery.
- A refused preparation-status command does not mean every broker operation is blocked: report the exact refusal and check each authorized operation separately. Review an upgrade plan before applying it. Lease coordination and migration preconditions are separate: a migration can treat any live or stale session, including your own, as a blocker, so apply it from the main checkout with no session open and carry the result into a session for the pull request.
- When a WASM export changes, the plugin build saves the new generated API declaration as a .wasm-bindgen file and restores the tracked declaration. Review and apply that diff, update the hand-maintained bg declaration and its crawl-worker copy, run the worker check:dts command, then rebuild and test the real WASM.

## Freshness

- Source digest: `038ebbb319751bc4e3cd7c525025fbedbf1658044bc45b20bf07362647c6d66d`
- Tracked source files: `381`
- Overrides applied: `True`
- Sections generated: `repo, workspaces, primary_workspace, commands, areas, entrypoints, caution_zones, generated_paths, dangerous_paths, navigation_recipes, summon, freshness`
