# Editorial style review and transformations

Goal: offer configurable, composable editorial observations and carefully bounded suggestions that help writers notice repetitive patterns. No rule establishes authorship or detector outcomes.

- [x] Advisory transition-opening gate with English and French presets, paragraph thresholds, bounded resources and explicit unavailable results.
- [x] Advisory paragraph punctuation repetition gate with a configurable Unicode mark and threshold.
- [x] Advisory English trailing `-ing` candidate and sentence rhythm gates, with bounded input and unavailable results.
- [x] Optional heuristic review of inline triple candidates, with explicit limits for ambiguous prose.
- [x] Observe structured list boundaries and review item counts across Markdown and explicitly attested EmDash adapters.
- [x] Portable news and opinion starter presets composed from existing and new gates, with independent English and French policies.
- [x] Source-bound, reviewable suggestions and explicitly applied candidate edits, with stale-input rejection and a documented gate revalidation step; never publish CMS changes automatically.
- [x] Preserve adapter capability and source-location boundaries across CLI, Markdown and EmDash; support telemetry without document text.
- [x] Documentation and agent skill usage; tests for configuration, locales, incomplete content, Unicode, quoted/code content and safe refusal.
- [x] Run strict lint, relevant Rust and real WASM tests, and validate each merged candidate.

## Delivery notes

- The style reviews and presets landed in PR #503; their findings are advisory and are not AI-authorship detection. Multi-sentence quotation handling is heuristic and incomplete quotations are not checked.
- Source-bound, explicitly reviewed patches landed in PR #505. This workflow accepts authored edits; it does not automatically turn a projected diagnostic into a safe source edit. Re-run gates after applying a candidate.
- Structured list support landed in PR #506. EmDash requires an explicit per-collection assertion of complete list projection.
- Candidate files are created with owner-only permissions on Unix (PR #509); redirected review output still follows the shell umask.
- Opt-in transition suggestions landed in PR #511. They cover a narrow set of paragraph openings; context detection for plain text remains incomplete and is not a semantic guarantee.
- The EmDash 0.9.2 package is pending publication; a merged PR does not mean the npm package has been published.

- Release validation and publication of the CLI and npm package remain outstanding.
