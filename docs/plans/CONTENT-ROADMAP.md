# Content quality roadmap

**Status: in progress.** Open items remain in the checklists below, including
independent evaluation and later source and factuality work.

This roadmap follows the original goal: portable, composable gates and presets,
adapters for different content sources, and fast feedback for agents. Source
and fact checking remain later priorities. A passed preset means its configured
checks passed; it is not an overall quality score.

## Evidence from the Aeptus article

A real EmDash article evaluated with `article-editorial` passed its three gates:
minimum word count, advisory editorial length, and advisory required metadata.
These checks did not evaluate readability, neutrality, sources or site SEO.
Metadata must be supplied when evaluating an exported Markdown file. An SEO
preset without a site adapter is not assessed. Use this case as a regression
scenario, not as a representative benchmark or an assessment of Aeptus overall.

### Preview CMS regression, 2026-11-11

A read-only copy of the mixed-component evidence-room article from the original
0.9.0 report was run through the current configured after-save hook and freshly
built WASM with an in-memory CMS host. The adapter retained all 18 Portable Text
blocks among 20 body items. Without a mapping for the two custom components,
body capabilities were unavailable; the article preset recorded its minimum-word
check as `not_checked` and persisted the assessment. Default evaluation did not
emit the reported unobservable page or empty-body rules. An experimental host
mapping that ignores those components retained the 18 text blocks and let that
minimal check pass; this experiment does not establish that ignoring those
components is correct for the site's renderer.

On local copies of the report's English webinar and French small-business records,
`descriptionFields: ["seo_description", "excerpt"]` selected the same nonempty
source value as the documented renderer fallback and marked that source observed.
No CMS content was changed. These checks did not inspect rendered HTML, the hosted
admin UI, translations, the deployed package, or every article; they do not prove
rendered metadata, full editorial quality, or a matching site snapshot.

## CMS review feedback — 2026-11-19

- [x] Preserve open-world CMS metadata coverage through the content gate and sandbox worker; unobserved fields do not produce missing-field findings and an incompatible worker is rejected. Older direct documents keep their prior semantics.
- [x] Report observed revision input coverage separately for body, description
  mapping and unobserved rendered pages; add an advisory fragment preset and
  an opt-in, source-mapped Portable Text literal Markdown link review ([#546](https://github.com/schiste/Aexeo/pull/546)).
- [x] Map description sources explicitly per collection, including nested
  revision data and SEO panel paths; a historical revision never substitutes the
  current SEO panel ([#548](https://github.com/schiste/Aexeo/pull/548)).
- [x] Allow an explicitly configured CMS field to select its own revision audit
  preset; missing or unmapped fields remain not checked. Exercise the actual
  configured plugin and WASM, and log scope and duration without field names
  ([#549](https://github.com/schiste/Aexeo/pull/549)).
- [x] Add opt-in independent fragment audits to configured saves and refreshes,
  with mapped findings, explicit unavailable results, aggregate telemetry and
  regression coverage using the WASM engine.
- [ ] Publish and install the tested npm package, enable the desired presets and
  revision access on the target host, then verify its connected MCP actions and
  permissions against a real revision. Repository support alone does not expose
  an action through an already connected host.
- [ ] Validate the host's historical metadata and component mappings, citations,
  language-specific coverage and redirect HTML against representative rendered
  builds. Style checks do not establish factual accuracy, currency or translation
  quality; source verification and independent evaluation remain separate work.

### Preview rendered-page spot check, 2026-11-19

Read-only comparison of the current CMS record for the evidence-room article
(`01KPTR098CK5JAGXXG08VX75V9`, published revision
`01M59ZV24D25M3AW25QC4SEQ8G`, locale `en-gb`) against its HTML at
`aeptus-preview.pages.dev`: the record contained 18 Portable Text blocks and
2 `askYourAi` components. All 23 text spans longer than 30 characters appeared
in the response HTML, as did the 10 body mark-definition URLs and 3 reference
URLs. The CMS `seo_description` appeared in the description meta tag; the HTML
also contained a canonical link, an H1 and a main landmark. These are substring
and element-presence checks, not proof of visible rendering, source support,
component fidelity or equivalence of the complete CMS and rendered snapshots.

The [2026-11-18 deployment run](https://github.com/Aeptus/website-and-content/actions/runs/44270994656)
reported five SEO004 missing-canonical findings on glossary category paths. On
2026-11-19, GET requests to those five paths at `aeptus-preview.pages.dev`
returned 200 HTML with `robots=noindex` and no canonical or meta refresh; the
`accounts-access` response advertised build revision
`9597807a32e8ed63387aab97ae681fe27e364710`, the same source commit as the run.
The source route contains a *conditional* redirect, so source inspection alone
cannot establish that these five responses redirect. The run deployed its built
`dist` after the audit, but a matching source commit alone does not bind the
served bytes or CMS snapshot to the audited files. Retain the open rendered
build validation above until the exact audit artifact and deployment are bound;
do not classify these findings as confirmed false positives or automatically
alter site content.

On 2026-11-19, the run's deployment log identified the per-deployment URL
`https://de5a021a.aeptus-preview.pages.dev`; unauthenticated GETs to all five
category routes there returned 403. The run's only retained GitHub artifact was
its deployment report, not the static files. The report records snapshot and
static artifact SHA-256 values, but those aggregate digests cannot independently
establish individual page bytes or their correspondence to the audit inputs.
An authenticated read of the deployment or recovery of the exact audited build
is still required to inspect those pages. No CMS content was changed.

## Delivery tracker

- [x] Project a numeric Portable Text reference across its complete contiguous
  link label, including formatting span boundaries. Keep mixed prose and numeric
  link labels and ambiguous overlapping links intact; preserve original literal
  text and coordinates. This does not validate citations or their sources.

- [x] Require an explicit offline or observed-link binding profile in both the
  Rust draft portable envelope and its independent JSON schema. Profile labels
  remain informational and do not authenticate execution or authorize publication.

- [x] Require an explicit non-timeout observation for the experimental Rust
  portable passing-verdict helper, including when a caller reports exit code
  zero. This aligns the host assertion with the independent comparator; neither
  authenticates execution or authorizes publication.

- [x] Add opt-in aggregate telemetry to the experimental portable verdict
  comparator, including comparison outcome, duration and optional-input
  presence without check IDs, paths or content. The comparator remains
  experimental and cannot authorize publication.

- [x] Add opt-in Arabic whitespace-token rhythm measurements and composable
  news/opinion starters. Their Arabic question-mark boundaries are explicit;
  unsupported characters remain not checked. These heuristics are not Arabic
  morphology, language detection or independently calibrated editorial quality.

- [x] Add opt-in German, Italian and Arabic transition-review starters and
  composable German and Italian news/opinion style starters. Rust and
  EmDash/WASM tests cover repeated transitions, missing paragraphs and source
  locations; these small, provisional phrase lists are not calibrated against
  an independent corpus and do not assess translation quality. Arabic sentence
  segmentation and diacritics remain only partially handled.

- [x] Extend the opt-in Portable Text literal Markdown review to recognize
  certain unfinished HTTP(S), root-relative, dot-relative and fragment links
  ([#630](https://github.com/schiste/Aexeo/pull/630),
  [#632](https://github.com/schiste/Aexeo/pull/632)). Findings remain advisory;
  the scanner can miss malformed links and does not validate targets or rendering.
- [x] Exercise skipped heading levels with the same synthetic fixture across
  Markdown, static HTML and EmDash/WASM, including source positions, empty or
  unavailable inputs and aggregate mapping telemetry
  ([#631](https://github.com/schiste/Aexeo/pull/631)). Real-host projection and
  independent editorial validation remain open.

- [x] Bind host-supplied link observation snapshots to experimental portable v2
  verdicts and compare their exact bytes independently. Snapshot provenance,
  freshness, native Aethyme interoperability and publication safety remain open.
- [x] Preserve the explicit `main` HTML scope in experimental portable v2
  verdicts ([#627](https://github.com/schiste/Aexeo/pull/627)). The real Aexeo
  CLI and independent Python comparator were tested. The synchronized Aexeo
  CLI at `40b66931cbba` and candidate Aethyme comparator at `449186435d00`
  also passed both portable interoperability scenarios, including HTML `main`
  scope and rejection of modified base, after, verdict and host options. The
  Aethyme integration in [#1870](https://github.com/schiste/Aethyme/pull/1870)
  remains pending validation and merge. Offline comparison does not establish
  source provenance, completeness or freshness, or authorize publication.
  On 2026-11-21 the Aexeo CLI at `acca16222721` and the resynchronized Aethyme
  #1870 candidate at `33780a61c3d8` passed the standard and HTML-main native
  interoperability scenarios, including rejection cases. The separately built
  #1908 candidate at `7abfe345ed41` passed the source-bound link-observation
  interoperability scenario. The Aethyme cross-process contract passed locally;
  its local fast-guards gate failed because the macOS TCP listener listing was
  unavailable in the agent environment. A separate macOS CI run was started;
  these results do not establish that either PR has merged, authenticate inputs,
  prove freshness or completeness, or make publication atomic.

- [x] Preserve exact HTML finding positions around unambiguous `<hr>`
  separators when source reconstruction matches the projection
  ([#625](https://github.com/schiste/Aexeo/pull/625)). Recovered or
  hidden markup remains unmapped; this does not establish browser visibility.

- [x] The built-in readability preset reports `not_checked` for sentences
  outside its conservative Latin character subset, retains findings on assessed
  sentences and measures skipped sentences ([#622](https://github.com/schiste/Aexeo/pull/622)).
  Standalone custom gates retain their default; policies inheriting readability
  pick up the guard. This does not
  provide language detection or validated segmentation for other scripts; emoji
  and other symbols can also cause a sentence to be skipped. Rust callers
  constructing `ContentGate::SentenceLength` must supply the new field.
- [x] Preserve exact HTML finding positions around recognized `<wbr>` void
  tags when the conservative source reconstruction matches the projection
  ([#621](https://github.com/schiste/Aexeo/pull/621)). Hidden or ambiguous
  markup remains unmapped; the mapping does not establish browser visibility.

- [x] Exclude format-unbound reviews and runs from policy-bound reviewed metrics,
  while retaining them in legacy aggregates and reporting their count. Format
  declarations are not authenticated.
- [x] Preserve the declared input format in new CLI feedback and reject known
  corpus format mismatches. Legacy artifacts without a format remain unverified;
  neither the format nor the fingerprint authenticates the evaluator.

- [x] Allow `paragraph-text` editorial corpora to replay through the same adapter
  and accept feedback export; input fingerprints do not bind adapter format, so
  hosts must verify it separately.

- [x] Keep EmDash content reports available when an optional synchronous
  telemetry callback throws; telemetry does not change the gate result.

The latest `@aeptus/aexeo-emdash` version observed in the npm registry on
2026-11-19 is 0.9.3. Rebuild the next package and WASM from the exact release
commit, publish it, and independently check that exact artifact against the
registry before considering the release delivered.

- [x] Reject duplicate JSON keys at any depth in supplied external gate results
  before evaluation and snapshot fingerprinting in both content CLI paths.
  This does not authenticate the result producer or establish factual accuracy.

- [x] Composable metadata length gate with per-field Unicode-scalar bounds,
  aggregate counts and distinct absent/unavailable reasons, shared by CLI,
  content-change and EmDash. Presence is controlled by `required_metadata`;
  metadata findings do not claim a body source location.

- [x] Optional, composable total character-count gate for text, Markdown, HTML,
  EmDash and change evaluation, with optional Unicode whitespace exclusion and
  explicit unavailable-text handling. Counts Unicode scalar values, not visual
  graphemes or linguistic words; thresholds remain host-defined.

- [x] Bound content CLI auxiliary files with per-file and shared byte budgets;
  optional local telemetry records limit events without paths or input data.
  Exceeding a limit aborts the command instead of returning a partial report.

- [x] Optionally explain the winning source key for each resolved preset gate,
  including inherited overrides and disables, without changing the existing
  reusable `--explain` output. Source keys do not authenticate authorship.

- [x] Resolve shared preset ancestors once per call and limit inheritance paths to
  64 presets, including when a cached ancestor is reused. No cross-call cache is
  retained, so updated sources are reflected on the next resolution.

- [x] Optional portable literal required-terms gate with explicit word and case
  behavior, bounded configuration, aggregate measurements and unavailable-text
  handling across text, Markdown, EmDash and content-change evaluation. It is not
  a semantic coverage assessment and has not been independently calibrated.

- [x] Reject content-change verdict paths that alias configuration, snapshot or
  confined content inputs. Filesystem changes concurrent with evaluation still
  require host coordination; this is not an atomic input/output transaction.

- [x] Report aggregate body findings withheld due to incomplete CMS projections; guard known native families against drift with a registry test. Unknown custom rules remain host-managed.

- [x] Withhold findings from known native rule families in CMS-only views, including old stored findings, except explicitly tagged content-preset results and two page-level checks supported by a complete body. Unknown host rule IDs remain the host evaluator’s responsibility; the native-family list must track new native rules.

- [x] Select content presets by exact locale and collection in both configured EmDash and the crawl worker, with explicit precedence and language-neutral fallback under host control. Locale does not prove the language of the text.
- [x] An optional host-provided historical-revision locale selects the EmDash
  audit preset and reaches its link observer; invalid or unavailable projections
  are not checked ([PR #698](https://github.com/schiste/Aexeo/pull/698)). The
  host remains responsible for binding the projection to the requested revision.
- [x] Preserve citation labels for literal gates and bind EmDash external gate
  inputs to the validated unmasked reference text, so a citation-only change
  cannot reuse a stale result ([PR #700](https://github.com/schiste/Aexeo/pull/700),
  [PR #701](https://github.com/schiste/Aexeo/pull/701)). Rhythm projections may
  still mask citation markers; input binding does not authenticate the evaluator.
- [x] Both EmDash assessment views display each gate’s scope and required capability, including an unknown state for older records.
- [x] Aggregate gate coverage and measurements for word count, required metadata,
  required headings and forbidden terms. Unavailable measurements remain absent.
- [x] Report the number of sentence and paragraph length violations even when
  findings are capped; emit opt-in anonymous aggregate length-check telemetry
  ([PR #137](https://github.com/schiste/Aexeo/pull/137)).
- [x] CI benchmarks and conservative budgets for evaluation, Markdown adaptation,
  HTML projection and evaluation, 50-item batches and fresh-process CLI runs, with p50/p95 telemetry. Fresh
  processes do not imply cold operating-system caches.
- [x] The text renderer walks ordered source locations once instead of rescanning
  them per finding. `python3 scripts/benchmark-content-report.py target/release/aexeo-cli`
  measures a synthetic 30,000-finding end-to-end report and emits local aggregate
  samples; it is an opt-in diagnostic, not a CI budget or a typical workload.
- [x] Performance budget validation requires every configured benchmark exactly
  once and rejects missing, invalid or non-finite measurements; independent
  content and EmDash example targets have distinct binary names.
- [x] Measure EmDash projection separately from projection plus evaluation on a
  synthetic 500-block snapshot, with local aggregate p50/p95 and CI budgets.
  These measurements exclude JSON parsing, WASM startup and network latency.
- [x] Single-parse Markdown link validation and a p50/p95 comparison against the
  two-parse baseline ([PR #47](https://github.com/schiste/Aexeo/pull/47)).
- [x] Opt-in per-gate timing telemetry for local content and crawls in resolved preset
  order; the event excludes gate IDs and content.
- [x] Opt-in paragraph-boundary reset for sentence variance windows, with
  explicit partial coverage when paragraphs cannot form a window. Existing
  cross-paragraph presets retain their behavior; adapter paragraphs do not
  necessarily distinguish list items from prose.
- [x] Add an opt-in prose-only rhythm scope with explicit, complete paragraph
  classification from Markdown, static HTML and EmDash. Keep existing preset
  behavior and public Rust callers compatible; unknown or partial classification
  must be `not_checked`, never interpreted as an empty list. Excluded items must
  break rolling windows, preserve original source positions and be counted in
  aggregate telemetry. Cover nested lists, adjacent prose, incomplete projections,
  text-only inputs and parity between CLI, content changes and EmDash/WASM.
  Implemented across PRs #666–#668, #670–#673 and #675; EmDash aggregate
  coverage telemetry is in #674. Host attestations for whole documents,
  fragments and historical revisions are independent and opt-in. Unknown
  custom components or incomplete body capabilities still yield `not_checked`.
  These advisory measurements do not establish authorship or factual accuracy.
  A synthetic Markdown check at `f8a956d64bc6` with one introductory sentence,
  four four-word list items, `window_size = 3` and `min_stddev_words = 6` yielded
  three advisory low-variance windows crossing or contained in the list. The
  existing paragraph reset prevents cross-paragraph windows but does not identify
  lists or assess whether a repeated list rhythm is intentional.
- [x] Configurable sentence-length heuristic and opt-in composed preset ([PR #29](https://github.com/schiste/Aexeo/pull/29)).
- [x] Optional, exact case-sensitive abbreviation exceptions for sentence length
  ([PR #125](https://github.com/schiste/Aexeo/pull/125)); they do not provide
  linguistic segmentation and can hide a true sentence end.
  Multi-period configured exceptions are recognized as a whole
  ([PR #699](https://github.com/schiste/Aexeo/pull/699)); this remains an
  explicit heuristic, not language-aware sentence segmentation.
- [x] Optional paragraph character-length gate for text without whitespace word
  boundaries ([PR #126](https://github.com/schiste/Aexeo/pull/126)), with
  Markdown/EmDash parity and explicit `not_checked` for missing paragraphs.
  It counts Unicode scalar values, not grapheme clusters or words.
- [x] Sentence word and character limits respect adapter block boundaries even
  when adjacent blocks have no inserted separator, preserving the first character
  and its location. They can opt into paragraph-only input,
  excluding headings while preserving adapter locations. Missing paragraph
  capability remains `not_checked`; existing presets keep their prior behavior.
- [x] Sentence length checks do not invent sentences from closing quotation marks or brackets after a delimiter; standalone quote and bracket tokens are excluded from sentence word counts. This remains a heuristic, not language-aware segmentation.
- [x] Optional sentence character-length gate ([PR #128](https://github.com/schiste/Aexeo/pull/128))
  uses the same configurable sentence boundaries and abbreviation exceptions;
  its Unicode scalar counts are not linguistic segmentation.
- [x] Per-example metadata in editorial corpora and metadata-bound human
  feedback ([PR #130](https://github.com/schiste/Aexeo/pull/130)). An additive
  `actual_unchecked_rate` reports expected and unexpected unchecked cases;
  the existing `unchecked_rate` retains its unexpected-only semantics. Neither
  the metadata hash nor supplied provenance proves review independence.
- [x] Reviewed corpus metrics require bindings to the current text, metadata
  and label; legacy unbound reviews are counted separately. Explicit feedback
  can be bound into a private corpus copy ([PR #132](https://github.com/schiste/Aexeo/pull/132)).
  The bindings do not prove reviewer independence; historical labels remain independent of a resolved policy.
- [x] Bounded agent lint-and-revise workflow and local telemetry guidance ([PR #30](https://github.com/schiste/Aexeo/pull/30)).
- [x] Configurable paragraph-length gate with Markdown and EmDash adapter
  capabilities ([PR #33](https://github.com/schiste/Aexeo/pull/33)).
- [x] Configurable link-count gate for Markdown and EmDash ([PR #35](https://github.com/schiste/Aexeo/pull/35)); this does not validate destinations.
- [x] Word-count gates treat known block ends as word separators, including
  adjacent projected blocks without whitespace; raw text retains whitespace counting.
- [x] Literal matching streams occurrences instead of retaining all offsets; claim
  anchors also respect known block boundaries. Word-level checks treat adjacent
  blocks as separate words even without inserted separators. Counts still include occurrences
  beyond the finding limit. Forbidden-term checks avoid constructing discarded
  findings or computing their positions. A synthetic 400 KB, 200,000-match case
  has a separate p50/p95 benchmark and conservative CI budget; it does not
  represent normal authoring latency. This does not verify factual support.
- [x] Configurable exact repeated-term check, with whole-word and substring modes
  ([PR #38](https://github.com/schiste/Aexeo/pull/38)).
- [x] Repeated terms can opt into per-character Unicode lowercase matching, with
  duplicate normalized terms rejected and source locations preserved; this is
  not full Unicode case folding. The default remains case-sensitive.
- [x] Offline link destination syntax and configurable scheme validation; this does not check reachability or link safety ([PR #46](https://github.com/schiste/Aexeo/pull/46)).
- [x] Configurable duplicate-heading gate with heading capability and aggregate
  counts ([PR #49](https://github.com/schiste/Aexeo/pull/49)).
- [x] Configurable opt-in heading length gate with Unicode scalar counting.
- [x] Markdown heading-node and EmDash heading-block locations for duplicate and length findings.
- [x] Truncated site crawls mark site-scoped gates `not_checked` while retaining partial findings; native link rules suppress incomplete graph conclusions.
- [x] Optional, bounded paragraph-count gate shared by Markdown and EmDash; unavailable paragraph capability remains `not_checked`.
- [x] Optional bounded duplicate-paragraph gate with measurements and Markdown / EmDash locations; incomplete capability remains `not_checked`.
- [x] Optional bounded HTTP link observations for Markdown or adapter-supplied
  JSON destinations, with non-public egress protection and aggregate telemetry.
  This advisory command is separate from content presets and does not establish
  destination safety, source reliability or long-term availability.
- [x] Optional blank-line-separated plain-text adapter exposes paragraph and
  block boundaries with original source lines, bounded paragraph count and
  explicit `not_checked` on overflow; the default raw-text mode is unchanged.
- [x] Composable advisory reachability gate in the core and Markdown CLI; network
  checks require explicit opt-in, and partial observations remain `not_checked`.
- [x] EmDash direct content API accepts snapshot-bound, explicitly supplied link
  observations, preserves partial results and maps findings to source spans.
- [x] Allow configured EmDash save and refresh workflows to use an explicitly
  installed link observer, preserving `not_checked` on incomplete observations.
- [x] Optional blank-line paragraph interpretation for `text/plain` content-change
  entries, retaining the raw-text default and source locations. Overflow remains
  not checked; opt-in telemetry records only eligible entry counts.
- [x] Content-change evaluation accepts opt-in, snapshot-bound custom gate
  results across entries and a trusted host extension; missing results remain
  not checked. Telemetry exposes only the configured custom-gate count.
- [x] Reuse the guarded HTTP client and run-scoped link observations across
  a content-change evaluation, within one global check budget. Aggregate
  telemetry counts only new checks; observations are discarded after the run.
- [x] Explicit static HTML content adapter for CLI and content-change verdicts,
  including bounded projections and optional host link observations. Prose findings
  can carry source positions for byte-identical text and a verified, well-nested
  literal HTML subset in CLI and content-change reports. Heading and link findings
  in that subset map to opening tags, including ordered reachability observations.
  Common semicolon-terminated named and non-control numeric entities map decoded characters to the opening ampersand; unsupported entities, hidden content and unsupported or malformed markup remain unmapped. Browser-rendered visibility remains unverified. CLI JSON and opt-in
  telemetry identify the mapping capability; telemetry also reports projection
  availability and aggregate heading, paragraph and link counts.
- [x] Preserve HTML source positions through explicitly structured tables;
  implicit or recovered table structures remain unmapped.
- [x] Single-content HTML CLI can explicitly select one non-hidden `<main>`
  ([PR #381](https://github.com/schiste/Aexeo/pull/381)). Missing or ambiguous
  selection is `not_checked`; default document scope is unchanged. Static markup
  does not establish browser visibility or rendered-page completeness.
- [x] Capture the selected scope in feedback and replay scoped HTML corpora,
  requiring the same scope for reports, reviews and blind packets
  ([PR #383](https://github.com/schiste/Aexeo/pull/383),
  [PR #384](https://github.com/schiste/Aexeo/pull/384)). This does not prove
  reviewer identity, independence or adherence to the rubric.
- [x] Map selected HTML findings to source positions only for a verified,
  well-nested subset, excluding repeated menu and footer occurrences
  ([PR #385](https://github.com/schiste/Aexeo/pull/385)). Unsupported markup
  remains unmapped. The content-change Rust API also supports main scope
  ([PR #387](https://github.com/schiste/Aexeo/pull/387)), and the CLI and
  experimental offline receipts bind and revalidate the selected scope. Static
  markup does not prove browser visibility or source freshness.
- [x] Support verified static HTML list observations in the single-content CLI
  ([#687](https://github.com/schiste/Aexeo/pull/687),
  [#688](https://github.com/schiste/Aexeo/pull/688)). Only requested list gates
  incur the extra projection; hidden, malformed or ambiguous structures remain
  `not_checked`. Static HTML is not proof of browser-rendered visibility.
- [ ] Integrate optional host observers with other adapters and continue adding
  independently validated editorial gates.
  The single-content CLI accepts source- and destination-bound host link
  observations for Markdown and HTML, with partial results and aggregate telemetry.
  Observation authenticity and freshness remain host responsibilities.
  Configured revision audits can separately opt in to a revision-specific link
  observer; results are projected from the historical snapshot, and provider
  errors and incomplete projections remain unchecked. The host is responsible
  for observer authorization, network policy, privacy, and freshness.
  Content-change hosts can also bind per-entry snapshots and revalidate their
  complete input fingerprint; omitted entries remain unchecked. EmDash observers
  receive route and locale context, and unchecked assessments distinguish a failed
  observation from an unavailable projection without exposing provider errors.
- [x] Explicit collection metadata uniqueness gate in core, CLI and EmDash/WASM
  ([PR #40](https://github.com/schiste/Aexeo/pull/40)); callers must supply the
  complete collection and missing metadata remains `not_checked`. Resource
  limits retain observed duplicates and report `not_checked`; measurements
  distinguish supplied items from the evaluated prefix.
- [x] Exact Markdown source locations and aggregate local mapping telemetry
  ([PR #42](https://github.com/schiste/Aexeo/pull/42)); transformed spans can
  remain unmapped, and projected finding coordinates are preserved.
- [x] Bound repeated UTF-16 source-position lookup in EmDash for many Unicode
  findings with a sparse index; keep a lightweight path for few findings and
  reject coordinate arithmetic overflow. Existing mapping telemetry stays aggregate-only.
  `cargo run --release -p aexeo-emdash-bridge --example emdash_source_mapping_bench`
  measures a long multilingual span with 250 findings using synthetic data.
- [x] Map exact lines in soft-wrapped Markdown containers, including block quotes,
  while keeping transformed text conservative and bounding extra mapping work.
  Existing opt-in telemetry reports mapped and unmapped counts without content.
- [x] Plain-text and paragraph-text CLI reports include validated source
  coordinates for positional prose findings, using the same mapping rules as
  content-change verdicts and opt-in aggregate mapping telemetry.
- [x] Map single-line Markdown entities and escaped punctuation, including text
  between multiple transformations, only when bounded reconstruction agrees
  exactly with the parser. Unsupported or ambiguous cases remain unmapped.
- [x] Extend bounded reconstruction to soft-wrapped Markdown containers when
  every line can be matched exactly; all lines share a single span budget.
- [x] Markdown invalid-link source locations and aggregate mapping telemetry
  ([PR #48](https://github.com/schiste/Aexeo/pull/48)); locations identify link
  occurrences, including reference uses.
- [x] Both EmDash findings interfaces flag records without a content-preset
  assessment, without assuming whether it was unconfigured or unavailable.
  Local aggregate telemetry distinguishes evaluated, missing and error reports;
  no report or findings-free result implies broad editorial quality.
- [x] Persist body capability coverage with each CMS evaluation and surface
  incomplete, unavailable or older unknown coverage in both dashboard paths,
  including evaluations without a content preset. Complete body coverage does
  not imply that rendered-page checks ran or that editorial quality passed.
- [x] CMS-record evaluation withholds legacy GEO, link graph and duplicate-page
  findings even when the body is
  complete: its synthesized sections and metadata
  are not the host's rendered page. Content-preset findings remain available;
  aggregate local telemetry counts withheld legacy findings without identifiers.
- [x] EmDash automatic field selection retains recognized prose but withholds body
  capabilities when another conventional body field is opaque; explicit host
  field selection can establish which fields comprise the body.
- [x] EmDash retains observed block boundaries for text checks when paragraph
  extraction is unavailable; paragraph-specific gates remain `not_checked`.
- [x] EmDash block/span coordinates and opt-in local aggregate telemetry
  ([PR #44](https://github.com/schiste/Aexeo/pull/44)); coordinates refer to the
  evaluated snapshot and use UTF-16 offsets.
- [x] Count reused EmDash link marks as separate destination occurrences, matching
  link counting and Markdown; emit opt-in aggregate destination-check telemetry
  ([PR #138](https://github.com/schiste/Aexeo/pull/138)).
- [x] An unreadable sandbox body-field selection preserves recognized prose but withholds body capabilities for save and refresh; aggregate coverage telemetry reflects the unavailable projection.
- [x] The EmDash sandbox accepts per-collection ordered description fields for
  save and refresh. Missing source fields remain unverified and aggregate
  description-coverage telemetry excludes values and identifiers.
- [x] Content-change hosts can supply one explicitly scoped, post-change collection
  with declared completeness. Its change ID, entry metadata, IDs and removed or
  renamed entries are checked; the host remains responsible for omitted unchanged
  records and revision consistency. Collection gates run once per change, including
  delete-only changes; site impact remains separately not checked. CLI telemetry
  reports aggregate collection size and the completeness assertion only.
- [x] Offline content-change verdicts include an optional Aexeo input fingerprint;
  CLI hosts can revalidate the current inputs and compare a fresh result before
  writing a verdict. Optional telemetry reports a match boolean only. This does
  not cover external callbacks, prove collection freshness or completeness, or
  protect consumers that do not perform the verification.
- [x] Expose offline verdict revalidation to Rust hosts: verify the input
  fingerprint, re-evaluate confined content, and compare all result fields
  except the evaluation timestamp. The CLI shares the comparison logic.
  External callbacks, build authenticity, collection completeness and freshness
  remain outside this offline mechanism.
- [x] For Rust hosts using a link observer without custom gate extensions, hash
  the ordered observer requests and responses with the change inputs, and compare
  the complete verdict against a fresh trusted evaluation. This detects changed
  observations even if gate checks are unchanged; it does not authenticate the
  observer, establish freshness, or protect consumers that ignore the extension.
  CLI revalidation also supports its own bounded link observer: it verifies
  inputs and network options before observation and compares fresh results.
  Custom extensions and other external evidence remain unsupported by that
  link-only binding.
- [x] Hash actual custom gate calls and responses, optionally with link
  observations, and compare fresh Rust verdicts. The CLI additionally binds the
  full supplied external-result snapshot and checks it before network work.
  Callers must provide trusted, current data; hashes do not authenticate it.
- [ ] Bind change verdicts to all decision inputs, including entry metadata,
  collection snapshots, effective policy, host options and external evidence,
  with a versioned interoperable verification contract. The current v1 subject
  binds content digests and base only; runners must independently retain and
  compare the other inputs or fail closed. Do not claim an optional extension
  protects legacy runners that ignore it.
  The Rust candidate verifier checks link-snapshot structure, change and source
  identities, and re-extracts ordered Markdown or scoped HTML destinations for
  supplied observations. Missing snapshots remain unobserved; this does not
  establish evidence authenticity, freshness or publication authorization.
  The optional cross-process test checks that the native Aethyme comparator
  also rejects unsuccessful, timed-out and missing fresh process status.
  The draft Python comparator validates offline host options and cross-checks
  the HTML scope extension against them; the Rust candidate verifier also
  compares that extension with its independently selected scope
  ([PR #703](https://github.com/schiste/Aexeo/pull/703)). These checks do not
  authenticate the options, evaluator, inputs or publication boundary.
  The Rust candidate verifier also compares validator profile and version to
  the reconstructed policy name and engine version; labels do not authenticate execution.
  The draft Python comparator also checks each verdict's validator version
  against the supplied host options and its profile against the supplied
  resolved policy name ([PR #709](https://github.com/schiste/Aexeo/pull/709)).
  This detects inconsistent labels; it does not validate the whole policy
  or authenticate the supplied artifacts.
  The independent draft Python comparator also requires an explicitly supplied
  successful fresh process exit and no timeout; these remain caller assertions,
  not authenticated execution or publication authorization.
  The draft v2 Rust envelope rejects inconsistent embedded verdict fields,
  summary counts and finding references before binding reuse. This validates
  internal structure, not the intended subject, required checks or authorization.
  An opt-in Rust check also matches the bound change subject and requires named
  passing blocking checks and caller-supplied validator labels and exit status;
  those assertions do not authenticate execution or authorize publication.
  A separate host-supplied streaming verifier checks each declared before and
  after state's bytes within limits, including inline equality; it does not
  establish absent states, input provenance, freshness or publication atomicity.
  Experimental candidate-bound verdicts can also evaluate a host-projected
  collection whose identities match the candidate (a subset is allowed when
  explicitly incomplete). The collection bytes
  are bound and independently revalidated; matching identities do not establish
  metadata correctness, provenance or completeness.
  The experimental CLI can strictly revalidate a prior draft portable verdict
  against a reconstructed candidate and a fresh evaluation, preserving an
  existing output on mismatch. Replay uses the same evaluator and does not
  establish trust, source freshness or publication atomicity.
  An experimental Rust helper reconstructs the candidate from a supplied base
  and change before comparing the decision binding and independently supplied
  content streams. It does not authenticate inputs, prove completeness or
  freshness, or make verification and publication atomic.
  The experimental candidate binding also supports host-supplied ordered link
  snapshots, with a distinct envelope and offline revalidation. Independent
  Rust/Python byte-binding checks do not establish source authenticity,
  freshness, publication safety, or a portable v2 contract.
  The experimental Rust link-snapshot receipt binds supplied ordered link
  observations in its manifest under a distinct schema and re-evaluates them.
  The CLI can create, bundle and revalidate this receipt with independently
  supplied link snapshots. The independent Python comparator also checks these
  receipts and bundles against separately supplied current artifacts; it does not
  recompute the link verdict, authenticate evidence or establish freshness.
  PR #315 added a standalone manifest, independently checked JSON
  Schema and an experimental Rust offline receipt. It is not yet a v2 verdict,
  a cross-implementation runner or integrated with EmDash; external
  observations, artifact formats, freshness and trusted build identity remain open.
  An opt-in CLI mode can write and revalidate Aexeo-specific offline receipts;
  its output is not a v1 verdict or a portable v2 verdict.
  PR #401 added an opt-in bundle transporting exact UTF-8 decision artifacts
  and a receipt for independent hash verification; revalidation still requires
  independently supplied current inputs. Bundles may contain private content
  and do not establish provenance, freshness or a portable v2 contract.
  CLI verdict revalidation rejects duplicate keys at every depth in the previous verdict before comparing or replacing output; this does not authenticate that verdict.
  CLI receipt mode rejects duplicate keys in changes, collections and previous receipts, and fields discarded by typed parsing in changes and receipts. Direct Rust hosts must independently ensure their typed inputs preserve all semantics.
  The independent offline-receipt comparator can also opt into validating the
  bound change against the v1 change schema, including date and URI formats;
  that mode verifies the subject as well and fails closed without the required
  schema dependencies. It does not verify content bytes or establish freshness,
  provenance, completeness or a portable v2 verdict.
  Its opt-in inline-content mode additionally compares the UTF-8 bytes of every
  before/after inline state with its declared digest and rejects file and URI
  references. An additional host-supplied content-file map can verify all
  referenced bytes without fetching URIs, with a shared size budget and strict
  coverage. It cannot verify freshness, source authenticity or publication races.
  An independent Python reference verifier compares bounded, explicitly supplied
  artifact files against the standalone manifest. An independent Python
  comparator also checks two Aexeo offline receipts against host-supplied
  artifact bytes and compares their verdicts except the evaluation timestamp.
  It rejects different decision manifests before reading artifacts and hashes
  each supplied artifact once; the host still must ensure stable, trusted inputs.
  Independent validated comparisons reject a Git origin paired with an opaque
  base and require an absolute content root for file references, as required by
  the v1 profile and the Rust evaluator.
  Candidate-envelope comparison can optionally validate both verdicts against
  a separately supplied fresh evaluation, the change subject and host-selected
  required checks and validator labels. It does not establish trust, freshness
  or publication atomicity; this remains experimental.
  The independent comparator can compare either a receipt or an explicit
  transport bundle on each side. It checks each bundle's embedded artifacts,
  then compares independently supplied current artifacts once per manifest check.
  Optional subject and content checks can reread files. This does not
  make the bundle's contents fresh, authentic or a portable v2 verdict.
  The independent comparator can require host-supplied exact validator name
  and version labels, with schema validation and aggregate-only telemetry. The
  labels are not authenticated and do not establish the evaluator's identity.
  The independent comparator can additionally require a host-supplied list of
  exact blocking check IDs to have passed; this does not establish evaluator
  authenticity, complete required coverage or publication authorization.
  An opt-in comparison mode also checks v1 verdict schema, timestamps, summary
  counts and status consistency. It rejects duplicate check IDs and dangling
  explicit finding references, while preserving optional v1 `check_id`.
  It does not produce a fresh evaluation,
  establish artifact freshness or authenticity, or authorize publication.
- [ ] Expand collection and adapter capabilities, including cross-adapter source mapping.
  The conservative HTML source mapper preserves positions through explicitly
  nested bidirectional inline elements, including Arabic text and links
  ([PR #705](https://github.com/schiste/Aexeo/pull/705)). Hidden or malformed
  structures remain unmapped; this does not validate browser rendering or
  translation quality.
  Conservatively map explicitly nested HTML `aside` and `address` containers;
  hidden and malformed structures remain unmapped. This is static source
  mapping, not a browser visibility guarantee.
  The conservative HTML source mapper supports explicitly closed definition
  lists with direct terms and descriptions, including links and Unicode.
  Ambiguous structures and content outside an explicit item remain unmapped.
  Removed-route reference scans retain only the bounded sorted output prefix and
  one truncation sentinel, preserving deterministic results and incomplete status.
  Conservatively map explicitly nested HTML blockquotes and their text and links;
  malformed or hidden structures remain unmapped, and static markup does not
  establish browser-rendered visibility.
  The conservative HTML source mapper supports explicitly nested and closed
  ordered and unordered lists, including links and Unicode; malformed list
  structures and hidden elements remain unmapped. An end-to-end change verdict
  test checks the reported source position.
  Shared Rust change-link snapshots now bind observations to change IDs, source
  digests and extracted link order, and reject unused supplied entries. Hosts
  must finish the observer after evaluation and establish evidence provenance and
  freshness independently; this does not add link evidence to offline receipts.
  Direct Rust collection uniqueness evaluation rejects ambiguous identities in
  the examined prefix, matching CLI and EmDash input validation.
  Conservatively map explicitly closed inline editing and emphasis markup
  (`del`, `ins`, `s`, `u`); ambiguous and hidden markup remains unmapped.
  Explicitly nested HTML figures and direct captions can retain source positions,
  including image alternatives and caption links; ambiguous markup remains unmapped.
  HTML image alt attributes and adjacent prose map to their source positions
  when the complete projection matches, including supported semicolon-terminated
  entities and CRLF normalization. Unsupported or ambiguous alternatives remain
  unmapped. This does not establish rendered visibility.
  Literal Markdown image alt labels map to their exact original positions for
  inline and reference images; transformed labels remain unlocated.
  Exact HTML source mapping also covers inline technical and emphasis elements
  (`code`, `kbd`, `samp`, `small`, `mark`) and citation markup (`sup`, `sub`,
  `q`, `cite`); block children of those elements remain unmapped.
  HTML with CRLF line endings can retain exact source locations when lexical
  reconstruction succeeds; bare CR and source maps exceeding the span budget
  remain unlocated.
- [x] Preserve preset and input-report provenance when binding human feedback; report policy-matched reviewed metrics separately from legacy metrics. Fingerprints do not authenticate people or bind external evidence.
- [x] Include a fingerprint of the CLI-resolved preset in corpus evaluation
  results and reject a policy change observed during a run. This identifies the
  evaluated configuration; legacy human labels may not be bound to it, and
  the digest does not identify executable versions or evidence.
- [x] Allow explicit acceptance of a conflicting human verdict into a new private
  corpus copy, preserving the former proposed label and review provenance. This
  supports curation; it does not itself establish reviewer independence.
- [x] Optional corpus strata report aggregate metrics and separately bound-review
  metrics by declared language or content category, including missing labels.
  The labels are unverified and results can contain repeated inputs.
- [x] Combine explicitly reported reviews from multiple reviewers into a version 2
  event log; preserve disagreement and report event-weighted override metrics and
  per-example disagreement counts. This does not authenticate reviewers or
  establish independence. Version 3 logs also preserve uncertain opinions,
  separate them from override metrics and report aggregate uncertainty.
- [x] Report normalized cross-split text overlap and optionally reject it before evaluation; this does not detect paraphrases or establish independence.
- [x] Provide an explicit local-only overlap inspection listing example IDs for curation, without exposing content in the aggregate report. IDs may be sensitive.
- [x] Prepare a private human-review packet without proposed labels, split or
  model decisions; import explicit pass/fail responses into a new corpus copy
  after matching text and metadata, retaining prior conflicting labels and
  aggregate-only telemetry. Uncertain opinions remain in the packet. This does
  not establish reviewer independence, rubric-policy equivalence or accuracy.
- [x] Combine multiple blind review packets as a version 4 review log, preserving
  uncertainty and measuring disagreement without inventing model verdicts.
  Matching text, metadata and rubric hashes are required; reviewer independence
  and the actual review conditions remain unverified. No labels are adjudicated.
- [ ] Independently reviewed, diverse evaluation corpus and held-out metrics.
  The project currently has one human participant and an assistant; their work
  must be attributed accurately and does not provide two independent human
  reviews. Single-reviewer data can still inform development, with this limit
  stated and held-out results reported separately.
  A synthetic, explicitly unreviewed starter and evaluator were delivered in
  [PR #32](https://github.com/schiste/Aexeo/pull/32); they do not establish
  production accuracy or substitute for independent review. The evaluator also
  accepts custom TOML and Markdown fixtures ([PR #37](https://github.com/schiste/Aexeo/pull/37)). The
  evaluator rejects exact cross-split duplicates and requires declared review
  provenance ([PR #43](https://github.com/schiste/Aexeo/pull/43)); it cannot
  establish reviewer independence or detect paraphrased leakage. Synthetic
  duplicate-heading, heading-length and link-destination fixtures also run as CI regressions;
  their unreviewed labels are not independent accuracy evidence. Local review-event
  logs can aggregate historical overrides without storing draft text ([PR #45](https://github.com/schiste/Aexeo/pull/45)).
- [x] Shared article/glossary readability preset delivered in
  [PR #34](https://github.com/schiste/Aexeo/pull/34).
- [x] Opt-in advisory metadata review-age gate with an explicit reference date;
  absent, malformed, unobserved and future dates are not assessed. Configured
  EmDash hosts can map fields per collection for save, refresh and revision
  audits, with aggregate projection telemetry. The host must refresh the
  reference date and identify a substantive review field. This is a scheduling
  signal only: it does not verify sources or establish factual currentness.
- [x] Forward explicitly mapped metadata and unavailable source fields from
  sandboxed EmDash saves and refreshes to the sidecar.
- [ ] Validate sandbox sidecar preset configuration, real host permissions and
  representative records before relying on the deployed workflow.
- [ ] Source and factuality evaluation after the basic gates are validated.
  A composable, advisory source-review-age gate checks an explicit reference
  date against a claim- and source-bound review record, with aggregate age
  measurements. It does not establish that the review occurred or facts hold.
  Structured evidence and external review records reject JSON with duplicate
  keys at any depth; ambiguous inputs are not checked, not treated as a pass.
  EmDash trusted hosts can opt into per-document evidence snapshots; failures remain unchecked and never fall back to a shared snapshot. This supplies inputs only and is not factuality validation.

An open PR is not yet delivered. Update this tracker when each change lands and
when its acceptance criteria are met, rather than inferring completion from a
partial implementation.

A bounded read-only retest of the five reported Preview CMS records is documented
in [CMS Preview validation](CMS-PREVIEW-VALIDATION-2026-11.md). It verifies the
CMS projection and reports its limitations; rendered-page and independent
editorial validation remain open.

## Priorities and acceptance criteria

Editorial findings from the shared content engine include advisory suggestions
for common length, repetition, heading, link and metadata checks. Suggestions
prompt review or policy adjustment; they do not assert that an automatic rewrite
is safe or that a heuristic establishes quality. Unavailable inputs remain
`not_checked` rather than generating advice for unobserved content.

1. **Explain the result.** Show configured-gate coverage, missing capabilities,
   blocking status, measured values and thresholds even when checks pass.
   Preserve machine contracts and distinguish unavailable measurements from zero.
   Done for aggregate coverage, word counts and metadata counts; required
   headings and metadata now report observed missing counts separately from
   unavailable fields. Expand to other gate types and more actionable remedies.
2. **Make basic editorial gates useful.** Add configurable checks for sentence
   and paragraph length, repeated terms, headings, links and prohibited wording;
   support different languages where measurement depends on language. Test
   precision and false positives on a curated, human-reviewed corpus, with each
   check individually switchable and severity configurable.
3. **Strengthen portable presets.** Publish documented, versioned website,
   article and glossary presets with small composable building blocks; specify
   inheritance, overrides and precedence. Include example configurations and
   regression tests for resolved policies and backwards compatibility.
4. **Improve adapters and scopes.** Define and document capabilities and source
   locations across Markdown, EmDash and website adapters. Support collection
   checks explicitly, and show what could not run rather than silently passing.
   A shared synthetic fixture exercises CLI Markdown, CLI static HTML and
   EmDash/WASM for matching gate outcomes, source locations, observed empty
   content and aggregate telemetry; unavailable EmDash content is separately
   checked. Static HTML projection does not establish browser-rendered visibility.
   Expand parity coverage to more gates, formats and real host projections.
   An opt-in heading hierarchy gate checks complete level sequences in Markdown,
   static HTML and EmDash. A configured initial level handles fragments under
   a template heading; unobserved or incomplete levels stay `not_checked`.
   Extend parity and real-host validation before making it a default.
5. **Make agent loops practical.** Ship discoverable skills and a single-item
   lint workflow with structured diagnostics, localized safe fixes and bounded
   retries. Re-evaluate after changes; do not treat a model's approval as ground
   truth. Validate on real authoring tasks and record false positives.
6. **Measure operational performance.** Maintain separate cold-start,
   single-item, batch and site benchmarks with repeatable inputs, p50/p95 and
   resource budgets in CI. Separate parsing, evaluation and adapter overhead.
   Current opt-in events include local stage timings and per-gate timings for local content and crawls;
   extend coverage to other adapters when useful and preserve privacy.
7. **Validate improvements independently.** Maintain a versioned, diverse,
   human-reviewed evaluation set and track precision, recall, unchecked rates,
   latency and reviewer overrides. Changes to rules or presets must be evaluated
   against a held-out set, not merely against their own acceptance rate.
8. **Add evidence and factuality last.** Explore source retrieval and semantic
   support only after basic gates are reliable. Benchmark CLEF or other models
   against reviewed examples, measure calibration and abstention, and keep
   uncertain cases for review. A small exploratory model test is not evidence of
   production accuracy or calibrated confidence.

## Telemetry principles

New features should define useful counters and timings, their cost and what they
exclude. Telemetry is local and explicitly enabled by default, contains no raw
content or identifiers, and never transmits automatically. Any future export
must be configurable, documented, and independently controllable; establish
retention, consent and redaction before enabling it.

### Feedback input matching

CLI JSON reports for complete local text include `input_fingerprint`: a SHA-256
of the original UTF-8 source and a hash of the string metadata map, if supplied.
Feedback capture compares both against the submitted inputs, rejecting mismatches.
Reports without the field remain accepted and are marked `report_binding: unverified`;
matching reports are marked `verified`. Export retains this distinction, and
review-log aggregates report both counts. Existing aggregate override rates still
include historical unverified events for compatibility. This check detects stale
inputs, not tampering: reports are unsigned, and matching does not establish the
policy, collection, evidence, site context, or reviewer independence. Truncated
or unavailable inputs do not receive a fingerprint.

### Feedback policy fingerprints

Local CLI JSON reports include a `resolved_policy_fingerprint` computed from
the policy used by that evaluation. Feedback capture and export preserve it;
corpus evaluation reports counts of historical events with the same, a different,
or no known policy. Existing override rates still combine these groups. A report
whose fingerprint conflicts with the policy resolved for a corpus run is rejected.
The `version: 1` hash uses SHA-256 over tagged JSON values: `n`, `t`, `f`, `i`,
`d`, `s`, `a`, or `o` for null, booleans, integer, float, string, array, or
object; strings and decimal integers have an eight-byte big-endian byte length,
floats use eight-byte big-endian IEEE-754 bits, and arrays/objects carry an
eight-byte item count. Object keys are UTF-8 length-framed in Unicode code-point
order. The fingerprint is unsigned and does not identify the executable, input
adapter, evidence, or other context; it does not establish reviewer independence.

### Source mapping coverage across adapters

Single-item CLI JSON reports include `source_mapping` counts for emitted
findings eligible for source positions, including when no trustworthy mapping is
available. Text reports summarize coverage when positional findings exist. The same aggregate counts appear in opt-in CLI telemetry; they omit
truncated findings and carry no source content. EmDash uses the same gate
eligibility definition. A mapped position identifies only the evaluated snapshot.
For scoped HTML, simple complete documents with a doctype and head can map
positions within `main`, including conservative comments, linked head resources
and raw-text head elements with literal `<` when no ambiguous HTML escape or
unrecognized end tag occurs. Ambiguous or unsupported markup remains unmapped.

### EmDash source mapping coverage

Single-item EmDash reports include counts of emitted findings eligible for source
location, mapped findings, and unmapped findings. Aggregate opt-in telemetry includes
the same eligible and unmapped counts, without content or identifiers. Both the
Block Kit document panel and React assessment show these counts, or explicitly
say they are unavailable for older reports. Counts precede suppressions and omit findings truncated by
the report limit; a location identifies the evaluated snapshot, not a later
edited document. Older WASM bridges do not provide the extra counts.

### Collection finding limits and identities

Collection uniqueness checks retain the full duplicate count while bounding emitted
findings and recording omitted findings. EmDash rejects blank or repeated routes,
matching the CLI collection identity requirement. A supplied collection must still
be complete; these checks cannot detect omitted records.

Known incomplete collections can be marked explicitly in the core, CLI and
EmDash API. Duplicate findings are retained, while the uniqueness result is
`not_checked`; a caller's completeness declaration is not independently verified.

Optional opaque source groups prevent known related excerpts crossing corpus splits.
Coverage is aggregated for the selected split; incomplete grouping and related
sources with different identifiers remain undetected and require human review.

Selected EmDash body fields distinguish unobserved fields from explicit nulls:
missing fields retain observed prose but make body capabilities unavailable.

- [x] Export one offline, source-bound link-observation template for a complete
  multi-entry content change; reject incomplete projections without replacing output.
