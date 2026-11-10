---
name: repo-act
description: Use after repo-onboarding or Explore when moving from orientation into execution planning, debugging checklists, and validation steps. Skip when only broad repo orientation is needed.
---

# Repo Act: Aexeo

## When to Use

- Load this after repo-onboarding when the next step is execution or validation planning.
- Skip it for broad repo overview questions with no action plan yet.

## Debugging Checklist

- Load repo-onboarding first if the repository or area is unfamiliar.
- Run Aethyme Explore on the user task before broad manual search.
- Verify answer candidates before concluding when trust_policy is not answer-safe.
- Identify the nearest fast test or validation command before editing.
- In a reused worktree, check package dependencies locally even when broker
  preparation is not configured: cleanup can remove ignored `node_modules`.
  Restore them with the package lockfile (`npm ci`) before Node tests; avoid
  an `npx` invocation that could install an unpinned missing executable.
- When reusing a squash-merged session branch, inspect ancestry and broker
  session-owned commits: old commits can still be replay candidates after a
  sync. Preserve a backup ref and reconcile the branch onto the current base
  with only the new commits before submitting; verify the resulting diff.
- Inspect caution zones before changing generated or vendored areas.

## Validation Checklist

- Run the fastest relevant test command first.
- Check likely entrypoints and impacted callers before finishing.
- After focused validation, run `sh scripts/clippy-strict.sh` for Rust changes.
  The CI applies extra lint rules beyond ordinary `cargo clippy -- -D warnings`;
  passing the latter alone does not validate the repository policy.
- For WASM bridge interface changes, synchronize the plugin and crawl-worker
  glue declarations and run `npm run check:dts --prefix packages/aexeo-crawl-worker`.
- For EmDash source-mapping tests, external diagnostics must be within the
  projected text. Use the valid position immediately after the last character
  to exercise an unmapped boundary; an out-of-range line is rejected earlier.
- Re-run other lint/build checks after the focused validation passes.
- Document any override-based maintainer note that affected the change plan.

## Useful Commands

- `fast_test`: `cargo test --workspace`
- `build`: `cargo build --workspace`
- `dev`: `cargo run`
