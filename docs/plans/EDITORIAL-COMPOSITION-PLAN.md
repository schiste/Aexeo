# Editorial composition and pacing

Status: in progress. A structural constraint is not a claim about authorship,
editorial quality, factual accuracy, or detector outcomes.

- [x] Define versioned TOML profiles and deterministic JSON Schema export.
- [x] Check structured content and preserve the declared field order in assembly.
- [x] Supply adjustable `news` and `opinion` starters and optional array-count ranges.
- [x] Add an advisory, composable content gate that binds structured observations
  to exact source bytes or an explicitly supplied complete text projection and
  withholds stale or unavailable snapshots.
- [x] Expose schema, assessment, assembly, source comparison and aggregate local
  telemetry to CLI agents, with a supported authoring skill.
- [ ] Validate each new path through the full workspace, WASM bridge and CI;
  deliver only once checks and review pass.
- [x] Add broader measurable sentence-rhythm signals with explicit language and
  segmentation limitations; field length alone does not prove one sentence.
  Opt-in reviews cover sentence-length variance, repeated and windowed openings,
  transition openings and similar sentence lengths. News and opinion presets
  compose advisory examples; thresholds are configurable, not calibrated
  measures of quality or authorship. See `docs/content-gates.md` for input,
  language, citation masking, resource and `not_checked` limitations.
- [ ] Exercise real authoring and editorial review; measure false positives on
  a representative corpus. Current synthetic fixtures are not accuracy evidence.
- [ ] Validate CMS host storage and runtime delivery of the structured snapshot,
  and compare the observed projection with rendered output. A host-declared text
  projection does not establish rendered-page equivalence. On 2026-11-23, the
  Aeptus website main branch at `67a4cdb049ed0d3fbc7fb9eb802a937f3001b123`
  declared `@aeptus/aexeo-emdash` 0.9.3 and configured neither an editorial
  content preset nor revision audits. The connected Preview MCP exposed no
  Aexeo audit action. Branch configuration does not establish the deployed
  version or its permissions; both require independent host verification.

Profiles are selected explicitly. A custom profile can be exported as a gate preset and composed with other presets,
so generation and validation share the same profile.
Built-in bounds are provisional, and may need adjustment for language, format,
and editorial judgment. A mismatch is not evidence of a quality problem.
