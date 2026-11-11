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
  Check the old remote branch before pushing: the new session may not own the
  previous session’s recorded push. If the broker refuses replacement, publish
  the new commit to a distinct branch through `broker advanced git`, open its
  PR through `broker advanced gh`, and retain the recorded checkout branch for
  submit and finish. Never overwrite the old branch without verified authority.
- Root `package.json` sets `type: module`: use ES module imports for root
  `.js` scripts and tests, or explicitly use `.cjs` for CommonJS. Run the
  script directly with Node to verify its entrypoint as well as its tests.
- Inspect caution zones before changing generated or vendored areas.

## Validation Checklist

- Run the fastest relevant test command first.
- Python test scripts with hyphens in their filenames are not discovered by
  `unittest discover`; run the relevant files directly, or use the repository
  check script that invokes them explicitly. Confirm the reported test count.
- Check likely entrypoints and impacted callers before finishing.
- After focused validation, run `sh scripts/clippy-strict.sh` for Rust changes.
  The CI applies extra lint rules beyond ordinary `cargo clippy -- -D warnings`;
  passing the latter alone does not validate the repository policy. Also run
  `cargo clippy --workspace --all-targets -- -D warnings`: the strict script
  covers libraries and binaries, while CI separately lints test targets.
- When changing CLI arguments, regenerate and verify the CLI reference with
  `cargo run -q -p aexeo-cli -- docs generate .` and `cargo run -q -p aexeo-cli -- docs check .`; CI checks generated documentation.
- Before EmDash `npm run test:wasm`, rebuild the plugin bundle after TypeScript
  changes (`npm run build:bundle`); `build:ts` alone leaves bundled entrypoints
  stale. Rebuild the WASM bridge too when Rust bridge code changes. When
  reporting results against real CMS snapshots, rebuild both the bundle and
  bridge from the identified commit even if neither source changed: ignored
  artifacts in reused worktrees may come from another revision.
- For WASM bridge interface changes, synchronize the plugin and crawl-worker
  glue declarations and run `npm run check:dts --prefix packages/aexeo-crawl-worker`.
- For EmDash source-mapping tests, external diagnostics must be within the
  projected text. Use the valid position immediately after the last character
  to exercise an unmapped boundary; an out-of-range line is rejected earlier.
- For protected site comparisons, verify the HTTP method and final response URL:
  a successful login page after a redirect is not an observed site page, and
  CMS access alone does not establish a matching rendered snapshot.
- For content file confinement, test symlinks, non-files and root replacement.
  Lexical checks and per-component no-follow opens alone do not prevent a
  concurrent directory rename from escaping the root; use an atomic platform
  primitive or fail closed, and run the tests on each supported platform.
- For JSON contracts, test omitted fields and explicit `null` separately when
  the schema distinguishes them: Serde `Option` normally collapses both.
- Re-run other lint/build checks after the focused validation passes.
- Document any override-based maintainer note that affected the change plan.

## Useful Commands

- `fast_test`: `cargo test --workspace`
- `build`: `cargo build --workspace`
- `dev`: `cargo run`
