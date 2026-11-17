# Editorial style review and transformations

Goal: offer configurable, composable editorial observations and carefully bounded suggestions that help writers notice repetitive patterns. No rule establishes authorship or detector outcomes.

- [x] Advisory transition-opening gate with English and French presets, paragraph thresholds, bounded resources and explicit unavailable results.
- [x] Advisory paragraph punctuation repetition gate with a configurable Unicode mark and threshold.
- [x] Advisory English trailing `-ing` candidate and sentence rhythm gates, with bounded input and unavailable results.
- [x] Optional heuristic review of inline triple candidates, with explicit limits for ambiguous prose.
- [ ] Observe structured list boundaries and review item counts across adapters.
- [x] Portable news and opinion starter presets composed from existing and new gates, with independent English and French policies.
- [ ] Source-bound, reviewable suggestions and opt-in safe transformations, with stale-input rejection and revalidation; never delete factual content or publish CMS changes automatically.
- [ ] Preserve adapter capability and source-location boundaries across CLI, Markdown and EmDash; support telemetry without document text.
- [ ] Documentation and agent skill usage; tests for configuration, locales, incomplete content, Unicode, quoted/code content and idempotence.
- [ ] Run strict lint, relevant Rust and real WASM tests, and validate the merge candidate.
