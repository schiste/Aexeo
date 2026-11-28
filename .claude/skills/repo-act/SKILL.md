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
- Inspect caution zones before changing generated or vendored areas.
- For parser-backed sentence comparisons, test paragraph separators included in
  the preceding or following sentence span, not only between spans. Confirm
  cross-paragraph pairs are not compared and insufficient observations are
  `not_checked`; verify finding positions after leading whitespace.

- For EmDash WASM import or bundling changes, test the built configured plugin
  with real WASM in workerd as well as Node; a sidecar-only Workers test
  does not exercise the host bundle. When reproducing a regression, verify
  the test also fails with the previous import and passes with the fix.

- When preparing an installed-package smoke test, stop on any script-generation
  or copy failure, verify the intended script is nonempty, and assert a known
  success marker. A shell can otherwise run an empty script successfully after
  a failed command, giving a false positive for the packaged artifact.

## Validation Checklist

- Run the fastest relevant test command first.
- Check likely entrypoints and impacted callers before finishing.
- Re-run lint/build only after the focused validation passes.
- Document any override-based maintainer note that affected the change plan.

## Useful Commands

- `fast_test`: `cargo test --workspace`
- `build`: `cargo build --workspace`
- `dev`: `cargo run`
