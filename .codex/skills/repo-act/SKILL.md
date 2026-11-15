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
- Before a guarded PR merge, fetch the full live `headRefOid` directly from
  `gh pr view --json headRefOid` and pass that exact value to
  `--match-head-commit`; never reconstruct a full SHA from an abbreviated log.
  Recheck the head and its checks if it changes before the merge.
- Before publishing even a new session, inspect its actual branch and any
  same-named remote branch; a different `--short-name` need not change the
  generated branch name. A stale remote can belong to an earlier session.
- When reusing a squash-merged session branch, inspect ancestry and broker
  session-owned commits: old commits can still be replay candidates after a
  sync. `git cherry` may report individual commits as missing when several were
  squash-merged together. Run `aethyme broker advanced representation scan
  --session <id>` and inspect its landing evidence before concluding they are
  unrepresented; record only the reviewed scan digest. Preserve a backup ref
  and reconcile the branch onto the current base
  with only the new commits before submitting; verify the resulting diff.
  Check the old remote branch before pushing: the new session may not own the
  previous session’s recorded push. If the broker refuses replacement, publish
  the new commit to a distinct branch through `broker advanced git`, open its
  PR through `broker advanced gh`, and retain the recorded checkout branch for
  submit and finish. Never overwrite the old branch without verified authority.
  Run a broker-coordinated push with `HEAD` from the session worktree: from
  another checkout, `HEAD` resolves there and the broker refuses the push.
- Root `package.json` sets `type: module`: use ES module imports for root
  `.js` scripts and tests, or explicitly use `.cjs` for CommonJS. Run the
  script directly with Node to verify its entrypoint as well as its tests.
- Inspect caution zones before changing generated or vendored areas.

## Validation Checklist

- Run the fastest relevant test command first.
- For report performance changes, compare release builds on the same reproducible synthetic input before and after, verify the full output stays equivalent, and report workload and timing variability rather than generalizing one measurement.
- Python test scripts with hyphens in their filenames are not discovered by
  `unittest discover`; run the relevant files directly, or use the repository
  check script that invokes them explicitly. Confirm the reported test count.
- For finding fields shown to editors, verify CLI JSON and text output and both EmDash Block Kit and React views; a field can survive serialization while being omitted from the interface.
- For partial HTML projections, validate source positions against the selected subtree and the corresponding ordered elements; matching extracted text alone does not prove that surrounding links or headings map to the same source nodes. Withhold locations when that correspondence cannot be established.
- Check likely entrypoints and impacted callers before finishing. For source-mapping changes, run adapter and single-content CLI tests, `content_source_locations` and `error_contract_integration`, and the entire affected content-change verdict test module, not only the new scenario: older cases can encode separate expectations about supported and unmapped transformations.
- After focused validation, run `sh scripts/clippy-strict.sh` for Rust changes.
  The CI applies extra lint rules beyond ordinary `cargo clippy -- -D warnings`;
  passing the latter alone does not validate the repository policy. Also run
  `cargo clippy --workspace --all-targets -- -D warnings`: the strict script
  covers libraries and binaries, while CI separately lints test targets.
- When changing CLI arguments, regenerate and verify the CLI reference with
  `cargo run -q -p aexeo-cli -- docs generate .` and `cargo run -q -p aexeo-cli -- docs check .`; CI checks generated documentation.
- EmDash Vitest runs in the plugin test host, whose filesystem is sandboxed;
  checks that must read repository files should run as a separate Node script
  outside the host. Test paths containing spaces as well as CI paths.
- Before EmDash `npm run test:wasm`, rebuild the plugin bundle after TypeScript
  changes (`npm run build:bundle`); `build:ts` alone leaves bundled entrypoints
  stale. In a fresh or cleaned worktree, check that the ignored WASM bridge
  artifacts exist and run `npm run build:wasm` before bundling if they are
  missing. Rebuild the WASM bridge too when Rust bridge code changes. When
  reporting results against real CMS snapshots, rebuild both the bundle and
  bridge from the identified commit even if neither source changed: ignored
  artifacts in reused worktrees may come from another revision.
- When smoke-testing a packed EmDash plugin in a clean project, install the
  host-compatible EmDash peer and its dependencies as well as the archive.
  Exercise the configured hook and real WASM; a missing peer import tests the
  fixture setup rather than package behavior. Do not confuse a CMS snapshot
  and mocked host with an end-to-end rendered-site validation.
- For WASM bridge interface changes, synchronize the plugin and crawl-worker
  glue declarations and run `npm run check:dts --prefix packages/aexeo-crawl-worker`.
- When EmDash produces data consumed by Rust, use a shared synthetic fixture:
  assert it matches the actual TypeScript exporter and pass those same bytes to
  the Rust or CLI consumer. Cover incomplete and unknown inputs separately;
  a fixture or CMS listing alone does not establish a matching rendered site.
- For partially observed metadata in core tests, put
  `unavailable_metadata_fields` in a `ContentContext` and pass it through
  `ContentEvaluationContext.adapter`; assert unknown fields stay distinct
  from observed missing fields and retain `not_checked`.
- For EmDash adapter configuration changes, test absent, valid, malformed and
  unreadable settings through both save and refresh. A failed settings read must
  not certify body capabilities via an automatic fallback; verify coverage
  telemetry and that recognized prose remains available.
- For EmDash source-mapping tests, external diagnostics must be within the
  projected text. Use the valid position immediately after the last character
  to exercise an unmapped boundary; an out-of-range line is rejected earlier.
- Keep real-content review packets awaiting people in a durable private directory,
  outside Git and temporary cleanup locations; verify restrictive permissions,
  preserve the corpus and rubric, and never treat placeholder labels as reviews.
- For protected site comparisons, verify the HTTP method and final response URL:
  a successful login page after a redirect is not an observed site page, and
  CMS access alone does not establish a matching rendered snapshot.
- For content file confinement, test symlinks, non-files and root replacement.
  Lexical checks and per-component no-follow opens alone do not prevent a
  concurrent directory rename from escaping the root; use an atomic platform
  primitive or fail closed, and run the tests on each supported platform.
- For CLI revalidation failures, assert the specific error exit and diagnostic,
  and that any previous output remains unchanged. A nonzero exit alone can be
  an ordinary failed editorial gate rather than a rejected revalidation.
- For receipt or manifest comparisons, test a supplied artifact changing
  between verification passes, and ensure different input snapshots are rejected.
  A matching digest does not establish provenance or protect later publication.
- For public Rust structs, check downstream struct-literal compatibility as well
  as serialized compatibility: adding a field can break callers even when
  Serde still accepts older JSON.
- For Python JSON Schema format checks, verify each required format checker is
  registered: optional dependencies can otherwise make validation silently skip
  it. Test malformed format values and fail closed when a checker is missing.
- For JSON contracts, test omitted fields and explicit `null` separately when
  the schema distinguishes them: Serde `Option` normally collapses both.
- Test unknown fields inside every tagged enum variant, including unit variants:
  Serde may ignore extra fields on an internally tagged unit variant even with
  `deny_unknown_fields`; an empty struct variant can enforce rejection.
- Re-run other lint/build checks after the focused validation passes.
- Document any override-based maintainer note that affected the change plan.

## Useful Commands

- `fast_test`: `cargo test --workspace`
- `build`: `cargo build --workspace`
- `dev`: `cargo run`
