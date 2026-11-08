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

## Validation Checklist

- Run the fastest relevant test command first.
- Check likely entrypoints and impacted callers before finishing.
- Re-run lint/build only after the focused validation passes.
- Run `sh scripts/check-code.sh` for code changes before claiming the repository gate passes; it includes `scripts/clippy-strict.sh`, whose production-code lint policy is stricter than generic workspace Clippy.
- After changing CLI commands or options, run `cargo run -q -p aexeo-cli -- docs generate .` and `cargo run -q -p aexeo-cli -- docs check .`; include the generated `docs/cli.md` changes.
- Document any override-based maintainer note that affected the change plan.

## Useful Commands

- `fast_test`: `cargo test --workspace`
- `build`: `cargo build --workspace`
- `dev`: `cargo run`
- `module_format_check`: `cargo fmt --all -- --check` after adding or moving modules; this also checks parent declarations.

- For a broker-coordinated `gh pr merge --match-head-commit`, use the full `headRefOid` from `gh pr view --json headRefOid`; an abbreviated SHA is rejected.

- If a broker push fails with an internal remote hook error, preserve the commit and inspect the error; retry through the broker without force. If it persists, report the rejection and keep the session for recovery.
