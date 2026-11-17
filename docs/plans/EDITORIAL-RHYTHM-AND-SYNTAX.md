# Editorial rhythm and syntax: implementation plan

These checks are advisory by default. They help authors review repetition; they do not establish authorship or guarantee a distinctive style. A legitimate parallel construction may be intentional.

- [x] Add sentence-length dispersion over bounded windows with a minimum sample, clear measurements and explicit `not_checked` outcomes. Support fixed thresholds and seeded, reproducible bounded thresholds; expose the resolved threshold and keep it stable during a revision loop.
- [x] Extend sentence-opening review to configured windows and optional ignored words without breaking existing consecutive checks.
- [x] Add optional language-specific syntactic review with explicit model and version, without a mandatory parser in the fast core or automatic model downloads. Report unavailable inputs and dependencies accurately.
- [x] Compose suitable opt-in presets and document calibration, language boundaries, telemetry, and the author's review workflow.
- [x] Validate configuration, CLI and applicable adapters, bounded execution, reproducibility and backward compatibility. Run meaningful end-to-end checks and update this plan with verified evidence.

The standalone variance and opening presets and the news/opinion profiles are advisory starting points, not calibrated thresholds. Real-model syntax smoke testing covered English; the general parser contract does not establish accuracy in any language. Human editorial calibration on representative content remains future work.

Verification: variance and opening rules passed their individual full CI and Aethyme gates, including shared CLI/WASM fixtures; syntax adapter passed offline CI and a real-model English CLI smoke test. The composed profiles have focused tests for parameters, observed findings and an insufficient variance window. The integration PR still requires its own CI before merge.
