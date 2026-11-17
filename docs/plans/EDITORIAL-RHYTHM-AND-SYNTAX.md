# Editorial rhythm and syntax: implementation plan

These checks are advisory by default. They help authors review repetition; they do not establish authorship or guarantee a distinctive style. A legitimate parallel construction may be intentional.

- [ ] Add sentence-length dispersion over bounded windows with a minimum sample, clear measurements and explicit `not_checked` outcomes. Support fixed thresholds and seeded, reproducible bounded thresholds; expose the resolved threshold and keep it stable during a revision loop.
- [ ] Extend sentence-opening review to configured windows and optional ignored words without breaking existing consecutive checks.
- [ ] Add optional language-specific syntactic review with explicit model and version, without a mandatory parser in the fast core or automatic model downloads. Report unavailable inputs and dependencies accurately.
- [ ] Compose suitable opt-in presets and document calibration, language boundaries, telemetry, and the author's review workflow.
- [ ] Validate configuration, CLI and applicable adapters, bounded execution, reproducibility and backward compatibility. Run meaningful end-to-end checks and update this plan with verified evidence.
