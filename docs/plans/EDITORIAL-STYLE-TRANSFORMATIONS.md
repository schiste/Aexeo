# Editorial style review and transformations

Goal: offer configurable, composable editorial observations and carefully bounded suggestions that help writers notice repetitive patterns. No rule establishes authorship or detector outcomes.

- [x] Advisory transition-opening gate with English and French presets, paragraph thresholds, bounded resources and explicit unavailable results.
- [ ] Additional advisory gates for trailing participle clauses, punctuation, rhythm and list patterns, with bounded resources and explicit unavailable results.
- [ ] Portable news and opinion presets composed from existing and new gates, with independent English and French policies.
- [ ] Source-bound, reviewable suggestions and opt-in safe transformations, with stale-input rejection and revalidation; never delete factual content or publish CMS changes automatically.
- [ ] Preserve adapter capability and source-location boundaries across CLI, Markdown and EmDash; support telemetry without document text.
- [ ] Documentation and agent skill usage; tests for configuration, locales, incomplete content, Unicode, quoted/code content and idempotence.
- [ ] Run strict lint, relevant Rust and real WASM tests, and validate the merge candidate.
