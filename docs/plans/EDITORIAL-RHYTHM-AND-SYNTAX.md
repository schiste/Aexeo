# Editorial rhythm and syntax: implementation plan

These checks are advisory by default. They help authors review repetition; they do not establish authorship or guarantee a distinctive style. A legitimate parallel construction may be intentional.

- [x] Add sentence-length dispersion over bounded windows with a minimum sample, clear measurements and explicit `not_checked` outcomes. Support fixed thresholds and seeded, reproducible bounded thresholds; expose the resolved threshold and keep it stable during a revision loop.
- [x] Extend sentence-opening review to configured windows and optional ignored words without breaking existing consecutive checks.
- [x] Add optional language-specific syntactic review with explicit model and version, without a mandatory parser in the fast core or automatic model downloads. Report unavailable inputs and dependencies accurately.
- [x] Compose suitable opt-in presets and document calibration, language boundaries, telemetry, and the author's review workflow.
- [x] Validate configuration, CLI and applicable adapters, bounded execution, reproducibility and backward compatibility. Run meaningful end-to-end checks and update this plan with verified evidence.

The standalone variance and opening presets and the news/opinion profiles are advisory starting points, not calibrated thresholds. Real-model syntax smoke testing covered English; the general parser contract does not establish accuracy in any language. Human editorial calibration on representative content remains future work.

Verification: variance and opening rules passed their individual full CI and Aethyme gates, including shared CLI/WASM fixtures; syntax adapter passed offline CI and a real-model English CLI smoke test. The composed profiles have focused tests for parameters, observed findings and an insufficient variance window. The integration PR still requires its own CI before merge.

## Configurable multilingual writing review — in progress

- [x] Compose opt-in writing review starters for general, French and English use from the existing advisory gates; document what the resulting checks cannot establish.

- [x] Add an opt-in adjacent-length percentage comparison with exact integer arithmetic, configurable threshold, explicit insufficient-input status, and an advisory starter.
- [x] Add configured colon-framing review and long-list starters for French and English; make phrases, conjunctions, separators and thresholds composable for other locales. Preserve the text and identify these as surface heuristics.
- [x] Add an optional local model-backed part-of-speech opening review with bounded input and unavailable results when observations are missing. Update the writing skill.
- [x] Exercise the supplied French sample, the core library suite, and offline syntax-adapter cases. These checks do not validate detection claims or grammar-model accuracy.
- [x] Run the opening-category adapter with a real compatible English model and the full Aexeo extension workflow; absence of a complete-coverage attestation produces `not_checked`. The smoke test used spaCy 3.8.11 and `en_core_web_sm` 3.8.0 with the previously built CLI from the same base, not a claim of model accuracy.
- [x] Supply a French opening-category example and smoke-test it end to end with spaCy 3.8.11 and `fr_core_news_sm` 3.8.0. A two-paragraph example with no comparable in-paragraph pair is `not_checked`; a synthetic single-paragraph example produces one advisory finding. Sentence spans containing paragraph-boundary whitespace are handled without comparing across blocks. These cases do not establish model accuracy.
- [ ] Evaluate representative human-reviewed content in each supported language and calibrate defaults.
- [x] Provide an optional comparison of configured literal phrase pairs against an explicit prior snapshot, bound to both drafts and the configuration. Test missing coverage and a changed prior snapshot end to end. This observes strings only; it cannot establish that one phrase replaced another.
- [ ] Design and evaluate language-specific idiom/register and semantic-equivalence review. A configured literal match cannot establish correct translation, concrete truth or an improvement in style. Until an appropriate assessment is supplied, do not claim that these requirements were checked.
- [ ] Review CI, validate the current merge candidate, and publish through the repository's normal release process.
