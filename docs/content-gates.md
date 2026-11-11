# Portable content gates

See [architecture and compatibility](content-architecture.md) for adapter boundaries and crawl completeness.

The content evaluator runs locally without a network or model. It can include the existing site audit through a site gate. It does not assess source support or factual correctness. A report records each gate as `passed`, `failed`, or `not_checked`; the last means an adapter could not supply a required capability.

For plain prose with blank lines separating paragraphs, use
`aexeo-cli content draft.txt --input-format paragraph-text --preset article-readable`.
LF and CRLF lines containing only whitespace separate paragraphs; soft-wrapped
lines stay together and locations refer to the original text. This interpretation
does not identify headings, links or other markup. The default `text` mode is
unchanged and does not claim paragraph structure. To bound memory, more than
100,000 paragraphs withhold the text input and affected gates report
`not_checked`; the usual input byte limit also applies.

```sh
aexeo-cli content article.md --input-format markdown --preset article --format json
printf 'Draft text' | aexeo-cli content - --preset glossary
aexeo-cli content --preset glossary --explain
aexeo-cli content --list-presets
aexeo-cli content --preset seo-website --site ./dist --no-text --format json
aexeo-cli content --preset seo-website --crawl-url https://example.org --crawl-max-pages 200 --format json
```

By default the CLI reads the supplied UTF-8 bytes as raw text, including Markdown syntax. With `--input-format markdown`, it parses CommonMark with GFM extensions and projects text and headings while excluding front matter, code blocks and HTML; link labels, image alt text and inline code text are included, matching EmDash, which keeps the text of code-marked spans. Word counts split on whitespace. Locations in raw-text mode refer to the supplied text. Markdown and EmDash findings refer to their extracted plain-text projection, not source line positions.

Markdown and EmDash adapters report where each heading, paragraph, list item and table cell ends. Those block ends terminate a sentence and a literal term cannot match across one. Soft-wrapped lines remain one sentence. Raw text has no known block structure.

Literal matching for `forbidden_terms`, `repeated_terms` and the claim of an evidence gate ignores whitespace differences: `best price` matches `best` and `price` separated by any run of whitespace without a blank line. Finding positions stay in the original text. Quote anchors in evidence sources remain exact.

`forbidden_terms` and `repeated_terms` accept `whole_words` (default `true`; set `false` for substring matches) and `case_insensitive` (default `false`). Case-insensitive matching lowercases each character, without full Unicode case folding. Word boundaries use Unicode alphanumeric characters and underscores. Thai, Lao, Myanmar, Khmer, CJK and Hangul characters do not act as word characters, because these scripts do not reliably separate words with spaces; a term in them matches inside surrounding text.

### Input limits

The CLI reads at most 16 MiB of content. Before parsing Markdown it rejects more than 100 nested block containers on one line, more than 100 unclosed link or image labels, or more than 1000 emphasis delimiter runs in one block. This conservative scan can overestimate nesting. Input beyond any limit is not evaluated: each gate that needs it reports `not_checked` with the limit as its reason, and a blocking gate exits 2. Rust callers of the Markdown adapter receive an error, and can test with `markdown_limit_reason` first.

A document gate keeps its first 1000 findings. It still fails, its source locations cover those findings, and its `findings_omitted` measurement counts the rest. A preset that resolves to no gates, including one whose gates are all disabled, is an error because it would check nothing.

Rust callers can evaluate a preset against a completed or partial `RuntimeAudit` with `evaluate_preset`. Its report retains the crawl status, truncation reason crawl-specific findings and the original full audit findings separately; inspect the crawl status before treating a site review as complete. The existing `crawl` CLI output is unchanged. Use `content --crawl-url URL --preset seo-website` for a preset report: the JSON envelope includes `crawl_status`, `truncation_reason`, `crawl_findings`, `audit_findings`, and `content`. A partial or failed crawl exits 2, regardless of the gate results; completed crawls use the normal blocking gate and audit severity rules. Site-scoped gates on a truncated crawl report `not_checked` because coverage is incomplete, while retaining any findings from the visited pages; these findings still require attention. A `not_checked` gate is not a pass. `--crawl-config`, `--crawl-max-pages`, and `--crawl-engine` configure this mode, which does not read stdin.

The built-in `seo-website` preset wraps the existing static SEO/GEO audit as one site-scoped gate and uses its existing configuration and policy. Pass `--site-config` to select a site config. It preserves the audit findings and their severities. The gate fails if the audit produces any findings, including warnings. A text-only adapter reports it as `not_checked`. For custom presets, `site_group` composes individual native website rule groups, for example:

```toml
[[gates]]
id = "site.html"
kind = "site_group"
group = "html"
```

Use `aexeo-cli content --list-site-groups` for supported group names. The existing site configuration's group switches, suppressions, and severity overrides apply. A group disabled by that configuration is `not_checked`. The `seo-website` preset remains a single site audit gate, preserving its existing complete findings, including deployment diagnostics.

The built-in `article` preset requires at least one whitespace-separated word. `glossary` inherits it and sets a maximum of 500 words. These deliberately minimal examples are starting points, not comprehensive editorial standards. `article-editorial` and `glossary-editorial` extend them with advisory word-count ranges and title/description metadata checks. Their thresholds are examples to override for your audience, language and content type; missing metadata is explicitly `not_checked`.

The [built-in TOML files](../crates/aexeo-core/presets) are embedded in native and WASM builds and can also be copied into another project as a starting point.

Use `--explain` to print the resolved preset as TOML, or add `--format json` for JSON.

## Editorial starter profiles

`article-quality`, `glossary-quality`, and `product-editorial` compose the
existing article or glossary checks with `editorial-structure`. Structure adds
advisory sentence and paragraph length and duplicate headings. The limits are
examples, not universal quality standards, and missing adapter capabilities are
reported as `not_checked`. The word-based gates are unsuitable for languages
without whitespace word boundaries without a tailored policy.

`natural-style-en` and `natural-style-fr` are optional, separate literal phrase
lists for editorial review. They do not detect AI authorship, measure human
style, or ensure any detector outcome. Their examples may be appropriate in
context. Inherit the content-type profile and one language profile explicitly;
there is no automatic language detection. Projects should replace or disable
`editorial.generic_phrases` according to their own editorial policy.

```toml
version = 1
name = "French product page"
extends = ["product-editorial", "natural-style-fr"]
```

### Repeated paragraph openings

The optional `writing-patterns` preset flags the third and subsequent paragraphs
that begin with the same three whitespace-separated words. Configure the
`repeated_paragraph_openings` gate using `opening_words` (1–16) and positive
`max_occurrences`. Matching lowercases and trims surrounding punctuation; it
does not infer meaning or identify authorship. Repeated openings may be
intentional, and languages without whitespace word boundaries need different
rules. A missing paragraph capability is `not_checked`. Findings point to the
paragraph start; measurements count paragraphs, openings evaluated or skipped,
and excess occurrences. If any paragraph is skipped because it has too few
usable whitespace-separated words, the gate reports `not_checked` and retains
findings from assessed paragraphs. This conservative result also applies to short
paragraphs that may be intentional; the check does not detect language or perform
linguistic segmentation. An explicitly empty paragraph collection still passes.
To bound resource use, an opening over 4,096 bytes or
more than 10,000 distinct openings stops the check as `not_checked`, retaining
findings from the portion already assessed.

## Custom presets

```toml
version = 1
name = "Product article"
extends = ["article"]

[[gates]]
id = "content.minimum_words"
kind = "word_count"
min = 100
max = 1200

[[gates]]
id = "style.forbidden_terms"
kind = "forbidden_terms"
terms = ["revolutionary", "guaranteed"]
severity = "warning"
blocking = false

[[gates]]
id = "structure.required_headings"
kind = "required_headings"
headings = ["Summary", "Examples"]

[[gates]]
id = "metadata.required"
kind = "required_metadata"
fields = ["title", "description"]
```

Save this as `product.toml`, then run:

```sh
aexeo-cli content article.md --preset product --preset-source product=product.toml --metadata metadata.json --format json
```

Heading names are matched exactly and case-sensitively. Raw-text input has no heading capability, so heading gates are `not_checked`; Markdown and EmDash provide it.

Metadata must be a JSON object with string values. Without `--metadata`, metadata gates are `not_checked`, rather than treating missing input as an empty set of fields. An empty object indicates that metadata was supplied but contains no fields. Adapters can also mark individual metadata fields unavailable. A required-metadata gate then remains `not_checked`, while findings for other known-missing fields remain visible. Its `unavailable_fields` measurement counts unknown required fields; `present_fields` counts known, nonempty fields. EmDash treats an unobserved description as unavailable, while an explicitly observed empty description is missing.

`extends` is a list of named presets. Parents are applied left to right; gates with an existing ID replace the earlier definition in place, and new gates append. `disable = ["gate.id"]` removes inherited gates and rejects unknown IDs. Supply each custom parent with a separate `--preset-source name=path`; built-ins are already available. Cycles, unknown parents, duplicate local IDs, unknown fields, invalid bounds and unsupported versions fail resolution. A preset's `name` is its report label, while its source name is the key used in `extends` and `--preset`.

Each gate may set `severity` to `error` or `warning` (default), and `blocking` to true (default) or false. Status always reports the gate result; the CLI exits 1 for a failed blocking gate, 2 if a blocking gate was not checked (even if another blocking gate failed), and 0 otherwise. Non-blocking gates still appear in the report, including when they could not be checked. Invalid configuration, invalid presets and unreadable inputs exit 2; 70 is reserved for internal errors in Aexeo. Content is unchanged unless `--fix` is passed explicitly.

The EmDash package exposes `evaluateContent` from `@aeptus/aexeo-emdash/content`; the Rust bridge and WASM API accept the same named source map and preset. A missing EmDash body is unavailable for text, heading, paragraph and link gates; an explicitly supplied empty body is evaluated as empty. The document adapter has no site capability, so site gates are `not_checked`. Configured-mode plugins can opt into a document preset for both save hooks and refreshes:

```ts
aexeoPlugin({
  contentPreset: { name: "article", byCollection: { glossary: "glossary" } },
})
```

For custom presets, add `sources: { custom: "...TOML..." }` and select `name: "custom"`. `name` is the fallback preset; `byCollection` selects a named preset for a specific collection on both save and refresh. The existing SEO findings remain enabled. A gate the document adapter cannot check causes an explicit incomplete-evaluation error rather than being silently omitted. For the sandboxed plugin, configure equivalent presets on the [sidecar worker](../packages/aexeo-crawl-worker/README.md) using its environment variables.

## Host-provided custom gates

A preset can declare an extension without embedding executable code:

```toml
[[gates]]
id = "style.project_rule"
kind = "custom"
extension = "my_project.rule"
severity = "warning"
blocking = false
[gates.parameters]
term = "example"
```

A Rust host implements `ContentGateExtensions` and calls `evaluate_with_extensions`, providing diagnostics in projected-text coordinates. The engine supplies each finding's rule ID and severity from the preset. Returning `None`, or running in an adapter that has no extension host, reports `not_checked`. Extension code executes in the host process and must be trusted; the preset is data and cannot execute a program by itself. CLI and direct EmDash callers may supply external results explicitly; configured EmDash hosts can install a runtime provider as described in the [installation guide](../packages/aexeo-emdash/INSTALL.md). The sandbox worker can call an explicitly configured [service binding](../packages/aexeo-crawl-worker/README.md). Built-in gates remain portable across these adapters.

### Explicit external extension results

The CLI and the direct EmDash content API can consume results produced by an
external tool. A preset declares `kind = "custom"` and names an extension; it
never runs a command. The operator invokes and trusts the external tool
separately. Results contain the **complete projected input**, including any
metadata, source text and evidence provided. Handle the files and data as
private content; no automatic transmission occurs.

To obtain the CLI input, run the same command with `--export-adapter-input`;
it prints JSON instead of evaluating gates. Capture it in a suitably private
file, run the external tool, and pass its JSON result array via
`--extension-results path.json`. For each result supply `extension`, a string
map `parameters`, the exact exported `input` object, and `diagnostics` with
`message`, one-based `line` and one-based Unicode-scalar `column` in projected
text. An empty diagnostic array means the external check passed. Missing or
stale inputs and changed parameters produce `not_checked`; malformed records,
invalid locations and exceeded size limits are errors. Source mapping is
provided where the adapter can map the projected position.

```json
[{"extension":"my_project.rule","parameters":{"mode":"strict"},"input":{"id":"draft.md","text":"Example","source_text":"Example","headings":null,"metadata":null,"evidence_sources":null},"diagnostics":[{"message":"Review this opening","line":1,"column":1}]}]
```

The EmDash `@aeptus/aexeo-emdash/content` API exposes
`projectContentInput(document, evidenceSources?)` and accepts the same result
array as the sixth `evaluateContent` argument. This is the direct API; configured
save hooks and the sidecar do not automatically invoke external tools. The
caller must collect results for each exact snapshot. The binding prevents stale
reuse but does not prove who produced the results or that they are correct.
Limits are 32 MiB for the JSON payload, 100 results, 1,000 diagnostics per
result and 4,096 UTF-8 bytes per message. No external program is sandboxed or
executed by Aexeo.

## Composable reachability gate

A portable preset can request an advisory observation gate:

```toml
[[gates]]
id = "editorial.external_links"
kind = "link_reachability"
blocking = false
```

Run Markdown evaluation with `aexeo-cli content article.md --input-format markdown
--preset custom --preset-source custom=policy.toml --check-link-reachability`.
The explicit flag enables network access only when this gate is present; otherwise
no link checks are made. `--link-max-checks` (1–100, default 20) and
`--link-timeout` (1–30 seconds, default 5) bound requests. Without the flag, or
when the adapter cannot supply complete destinations, the gate is `not_checked`.
Valid relative links and non-HTTP schemes are outside this external gate. Malformed
or scheme-relative destinations and refused, failed or budget-exhausted requests
leave coverage incomplete.
A 404/410 response is a finding to review, not proof of permanent failure. If
some checks are inconclusive, findings for observed responses are retained and
the gate is `not_checked`. Markdown source locations point to link occurrences.
The optional local telemetry event contains counts and elapsed time, not URLs;
external servers necessarily receive requested URLs. Other hosts may supply
observations through the core context, bound by the host to the same immutable
snapshot and destination order. Aexeo does not authenticate those observations.

### EmDash host observations

The `@aeptus/aexeo-emdash/content` API can supply the same gate without making
network requests in WASM. `projectLinkDestinations(document)` returns `null` if
link coverage is unavailable, or an object containing the exact document snapshot
and destinations in occurrence order. Send the destinations to a trusted host
checker with its own explicit network policy, then attach one observation per
occurrence (`reachable`, `not_found`, `not_checked`, or `not_applicable`):

```ts
const projection = await projectLinkDestinations(document);
if (projection) {
  const observations = await trustedHostChecker(projection.destinations);
  const report = await evaluateContent(document, "custom", { custom: presetToml },
    undefined, onTelemetry, undefined, { ...projection, observations });
}
```

Configured EmDash save and refresh workflows can also use an explicitly installed
runtime observer. The configured preset must include the reachability gate:

```ts
const plugin = createPlugin(
  { contentPreset: { name: "custom", sources: { custom: presetToml } } },
  { linkObservations: async ({ destinations }) => trustedHostChecker(destinations),
    onLinkProviderTelemetry: (event) => recordAggregateEvent(event) },
);
```

The callback receives a copy of the ordered destinations. Aexeo makes no network
requests itself; the trusted host must apply its own egress restrictions, timeouts,
request budget and consent policy. The observer runs only when the resolved preset
requires reachability. Missing projections, invalid results or observer failures
retain the initial report with the gate `not_checked`. Runtime telemetry reports
only status and duration, without URLs or content.
An observer must use `not_found` only for an observed 404/410 and keep timeouts,
authentication failures, refused URLs and exhausted budgets `not_checked`.
The batch is accepted only for the exact unchanged document and ordered
projection, with complete link capability and matching counts. A stale batch
leaves the gate `not_checked`; it does not authenticate the observer or guarantee
that a web response is still current. The projection and supplied batch contain
full content and URLs: keep them private, and provide them only to a host you trust.
Opt-in telemetry contains aggregate checked, not-found and unchecked counts.

## Optional external link observations

`aexeo-cli link-reachability article.md` explicitly contacts absolute HTTP(S)
destinations from Markdown. For other adapters, use `--input-format json` with
an array of destinations in occurrence order. The JSON output reports one-based
occurrence indices, HTTP observations and reasons without echoing destination
URLs. `--max-checks` (1–100, default 20) bounds unique destinations and
`--timeout` (1–30 seconds, default 5) bounds each check including redirects.
Duplicate URLs share a result; fragments are not checked. Results are advisory:
404/410 are observed responses, not proof that a destination is permanently
broken; authentication, rate limiting, transport errors, refused addresses,
unsupported inputs and budget exhaustion remain uncertain or skipped. HEAD is
tried first, with GET fallback for 405/501; bodies are not consumed.

Private-network addresses and credentials in URLs are refused, proxies are
disabled, and every redirect is checked (with a five-hop limit and no HTTPS to
HTTP downgrade). Requests may expose the URL, including query parameters, to
the destination. This command does not run as part of normal content gates,
validate source reliability or claim that a destination is safe. `--telemetry`
emits aggregate local counts and elapsed time without URLs or content.

## Corpus policy provenance

The local corpus evaluator resolves the CLI preset before and after a run and
includes `resolved_policy_sha256` and `resolved_policy_hash_format` in its JSON
result. The digest hashes the resolved preset as UTF-8 JSON with sorted object
keys, compact separators and ordered arrays; a changed observed policy aborts
the run. It excludes tool version, input format, source snapshots and execution
environment, and cannot detect a temporary change that is reverted between
checks. Retain those separately when comparing runs. Existing human labels and
historical review events are not bound to this fingerprint; they remain useful
for comparison across policies and are not proof of independent review.

### Comparing corpus categories

Examples can optionally declare up to eight `strata` categories, such as
`"strata": {"language": "fr", "content_type": "article"}`. Names and values
must be 1–64 ASCII letters, digits, `.`, `_`, `:` or `-`. The evaluator adds
per-category counts, precision, recall and unchecked rates for all examples and,
separately, reviews bound to their text, metadata and expected status. Each
dimension also reports examples and bound reviews without a label. An absent `strata` object is
valid; these labels do not affect the evaluated content or policy fingerprint.

Labels are supplied by the corpus author, are not independently verified or
bound to the human review, and appear verbatim in the local JSON output. Use
coarse categories, not names, IDs or sensitive data. Small groups are not
representative; these metrics include repeated inputs, so also inspect the
existing unique-input metrics and source-group coverage. Corpus results are
local and do not transmit data automatically.

### Combining independent reported reviews

When several people review examples, create a separate feedback note for each
person and combine explicit pass/fail/uncertain notes against the same corpus:

```sh
python3 scripts/content-review-note.py combine-reviews --corpus corpus.json \
  --note review-a.json --note review-b.json --output reviews.json --telemetry
python3 scripts/evaluate-content-gates.py corpus.json --binary target/release/aexeo-cli \
  --split development --review-events reviews.json
```

The output is a version 2 review-event log, or version 3 when a note is
uncertain, accepted by the current corpus evaluator's `--review-events` option.
Older evaluator versions may reject version 3. It retains each reported verdict,
validates each note
against the corpus text and metadata, and rejects repeated example/reviewer pairs
(reviewer identifiers are compared after trimming outer whitespace). It does not
modify corpus labels, resolve disagreement, authenticate reviewers, or prove their
independence. Keep the original private notes for context: exported events
exclude reasons and other private notes.

For versions 2 and 3, review metrics add `reviewed_examples` and `disagreed_examples`
for the selected split. An example counts as disagreed only when both pass and
fail were reported. Version 3 also reports `uncertain_reviews` and
`uncertain_examples`; an example with only uncertain reviews is included in
`reviewed_examples`, but uncertainty is excluded from override rates, agreement
and resolved-unchecked counts. Policy breakdowns include uncertain events.
Existing event and override counts are per review, so an
example with more reviewers has greater weight; disagreement compares reported
human statuses and does not account for differences in model policy or time.
The log contains reviewer identifiers and input hashes, and must remain private.
The command creates a new file with private permissions and refuses overwrite.
Opt-in local telemetry contains only duration and aggregate counts, including
uncertainty for version 3, with no text, identifiers or reasons. It does not
transmit data.

### Resolving a human review disagreement

A reviewer can explicitly replace an unreviewed proposed corpus label when their
pass/fail verdict differs. First capture the human feedback and check its contents;
then create a new private corpus copy:

```sh
python3 scripts/content-review-note.py bind-review \
  --corpus corpus.json --note review-note.json \
  --accept-human-label --output reviewed-corpus.json --telemetry
```

Without `--accept-human-label`, a disagreement is rejected. The new copy records
`previous_expected` alongside the reviewer, method, input hashes and final label;
the original corpus is untouched and existing reviews cannot be overwritten.
Uncertain verdicts, stale input or metadata, synthetic regression fixtures and
missing provenance are still rejected. Output creation refuses to overwrite a
file and uses private permissions. The optional local telemetry records only the
operation, duration and whether the label changed, not the reviewer or content.

This records a reported human decision; it does not authenticate the reviewer,
resolve disagreements among reviewers, or establish an independent gold label.
Keep the original corpus and note securely when a complete audit trail is needed.

### Preventing source-group leakage

Corpus examples can carry `source_group`, an opaque identifier shared by excerpts
from the same document or other related source. Use 1–128 ASCII letters, digits,
periods, underscores, colons or hyphens, and avoid personal or confidential data
in identifiers. The evaluator and both feedback exporters reject a group appearing
in both development and holdout, even if its excerpts differ. The evaluation report
includes grouped and ungrouped example counts and the number of distinct groups
for the selected split, without group identifiers. Grouping is optional for legacy
corpora; absent or incorrectly assigned groups, paraphrases and related sources
cannot be detected automatically and do not establish independence.

## Local latency measurement

For local content and crawl evaluation, `--telemetry` also emits `gate_timings_ms`, an
array in resolved preset order. Values measure the execution of each gate,
including result construction, and omit preset validation, source mapping and
output rendering. The event contains no gate IDs or content. Instrumentation
adds clock and observer overhead; timings are diagnostic rather than a benchmark.
For crawl evaluation, gate and evaluation timings exclude crawling itself.

Run `cargo run --release --locked --offline -p aexeo-core --example content_bench` to measure the content gates alone and Markdown parsing plus evaluation over a synthetic 29 KB document. It reports p50 and p95 across 100 iterations; figures depend on hardware and concurrent load and exclude CLI process startup. With `--json` it also emits machine-readable p50, p95 and average measurements for individual evaluation, Markdown parsing plus evaluation, and batches of 50. `scripts/check-performance.sh` enforces generous average-time and p95 regression ceilings; these are not latency guarantees for other workloads. The fresh-process CLI benchmark includes startup and evaluation of a tiny text input, but does not flush operating-system caches and is not a cold-cache measurement.

## Explicit safe fix

The `final_newline` gate requires the original source text and reports a non-empty file without a final newline. Add `[[gates]]`, an `id`, and `kind = "final_newline"` to a preset to enable it. Run `aexeo-cli content article.md --preset custom --preset-source custom=rules.toml --fix` to append a newline when needed. The fix preserves existing CRLF line endings, reports its gate ID in `applied_fixes`, and is idempotent. It requires a local regular file, rejects symlinks and, on Unix, files with multiple hard links, and compares the open file with the version read before appending. It does not change prose. The EmDash adapter has no original file source, so this gate is `not_checked` there.

## Literal evidence anchors

An optional `evidence_anchors` gate checks that a literal claim appears in the projected content and a literal quote appears in a named, caller-supplied source snapshot:

```toml
[[gates]]
id = "citation.anchor"
kind = "evidence_anchors"
source = "study"
claim = "The count is 14."
quote = "14 participants"
```

For the CLI, supply the UTF-8 snapshot with `--evidence-source study=./snapshot.txt`. The EmDash API accepts an optional fourth argument: `evaluateContent(document, preset, sources, { study: sourceText })`. The configured plugin accepts `evidenceSources` in its content preset options; the crawl worker accepts a JSON string map in `CONTENT_EVIDENCE_SOURCES_JSON`. Inputs are explicit: no source is fetched automatically. Missing source data yields `not_checked`, while missing literal text yields a finding. Matching is exact and case-sensitive.

A pass establishes only that both strings are present: it does **not** establish that the quote supports the claim, that the source is reliable, or that the claim is true. Callers must version their snapshots and use independent review or a suitable external verifier for those judgments. Trusted Rust hosts can implement additional checks through `ContentGateExtensions`; unavailable extensions remain `not_checked`.

## Structured facts

For a claim manually mapped to a structured JSON source, `structured_fact` checks that the exact claim occurs in the content and that the JSON value at the configured pointer equals the expected typed value:

```toml
[[gates]]
id = "study.count"
kind = "structured_fact"
source = "study"
claim = "The study had 14 participants."
pointer = "/sample/count"
expected = 14
```

Supply the source using the same evidence snapshot interfaces described above. A missing source, invalid JSON or missing pointer is `not_checked`; a different value or absent claim is a failure. Comparison is typed JSON equality, not numerical inference. The preset author is responsible for mapping the claim to the correct field and value; the engine does not understand the claim, assess source reliability, or verify reality. Version and review both the preset and source.

## External source reviews

`evidence_review` consumes a caller-supplied semantic and reliability assessment. The assessment is a separate JSON snapshot, bound to the exact claim and source text. This allows an editor or an external decision model to contribute a verdict without embedding a provider in the portable engine:

```toml
[[gates]]
id = "study.review"
kind = "evidence_review"
source = "study"
review = "study-review"
claim = "The count is 14."
```

```json
{"claim":"The count is 14.","source_text":"Study observed 14 participants.","support":"supported","reliability":"reliable","reviewer":"editor-123"}
```

Supply both named snapshots, for example `--evidence-source study=source.txt --evidence-source study-review=review.json`. `support` accepts `supported`, `contradicted` or `uncertain`; `reliability` accepts `reliable`, `unreliable` or `uncertain`. Only supported plus reliable passes. Contradiction or unreliable source fails; uncertainty, missing inputs, malformed records and reviews of a different claim or source yield `not_checked`. The reviewer identifier must be nonempty. An optional `provenance` object can hold string metadata; it is informational and does not authenticate the review. A changed source requires a new review.

The [source review process](source-review-process.md) describes how an application can record a claim-specific human assessment and when to reassess it.

The review itself is **trusted input**: Aexeo does not authenticate the reviewer, run an NLI model, calibrate its confidence, or independently verify the stated judgments. Applications must choose and evaluate their reviewer, protect the assessment's provenance, and decide when a human should review it. Exact text binding prevents accidental reuse against different input, but does not prevent a forged review.

An optional [Jev adapter](../adapters/jev/README.md) produces this review format from an explicit remote model call and a separately supplied source-reliability judgment. It is not part of the default offline engine.

### Paragraph count

`paragraph_count` checks optional inclusive `min` and `max` bounds against the
paragraphs provided by the adapter. At least one bound is required; zero is a
valid bound and `min` cannot exceed `max`. Markdown paragraphs and EmDash normal
blocks are counted, not headings. The report includes `paragraph_count`,
`min_paragraphs` and `max_paragraphs` when available. A missing paragraph
capability produces `not_checked`; an observed empty body has count zero. The
finding applies to the whole document, so no precise source location is claimed.
Counts are structural hints, not a measure of prose quality.

```toml
[[gates]]
id = "editorial.paragraph_count"
kind = "paragraph_count"
min = 2
max = 12
blocking = false
```

### Paragraph length

`paragraph_length` counts whitespace-separated words in each paragraph supplied
by the adapter. Markdown paragraphs and EmDash normal blocks are supported;
headings are excluded. Plain text does not provide paragraph structure and
therefore produces `not_checked`, not an inferred pass. Locations refer to the
adapter's projected text, not necessarily the original source. The gate reports
`paragraph_count`, `longest_paragraph_words`, `paragraphs_exceeding_limit`, and the configured `max_words`.

```toml
[[gates]]
id = "editorial.paragraph_length"
kind = "paragraph_length"
max_words = 100
blocking = false
```

### Link count

The `link_count` gate requires a configured minimum number of hyperlinks:

```toml
[[gates]]
id = "editorial.minimum_links"
kind = "link_count"
min = 1
blocking = false
```

Markdown links and EmDash link marks are counted, including links split across
adjacent spans. Images are not counted. This is a presence check only: it does
not validate destinations, check network availability, or assess sources.
When an adapter cannot supply link data, the gate reports `not_checked`.

### Sentence length

The opt-in `sentence_length` gate limits whitespace-separated words per sentence. `article-readable` and `glossary-readable` combine their editorial starter with the shared `readability` preset, which adds it as an advisory 30-word limit:

```toml
[[gates]]
id = "editorial.sentence_length"
kind = "sentence_length"
max_words = 30
delimiters = ".!?。！？"
blocking = false
```

`delimiters` lists the characters that end a sentence and defaults to `.!?`. A decimal point between ASCII digits does not end one, and neither do Markdown hard breaks. Adapter block ends always do, so unpunctuated bullets, headings and table cells are measured separately. This is a heuristic, not linguistic segmentation. By default abbreviations end a sentence. Optionally set `abbreviations = ["Dr.", "M."]` to keep the period of an exact, case-sensitive, whitespace-delimited abbreviation inside a sentence. This also applies at a true sentence end, so choose exceptions for the content carefully; adapter block boundaries still end sentences. Multi-period abbreviations and words attached to opening punctuation are not recognized by this option. Languages without whitespace-separated words need tailored rules. Measurements report the limit, sentence count, longest sentence and `sentences_exceeding_limit`; absent text is `not_checked`.

Set `paragraphs_only = true` on either sentence-length gate to measure only
adapter-supplied paragraphs, excluding headings and other non-paragraph blocks.
Each paragraph ends a sentence. This option requires the paragraphs capability;
when unavailable the gate is `not_checked`, while an observed empty paragraph
collection is assessed as empty. Markdown and EmDash provide paragraph positions
when their projections are complete; plain text needs explicit paragraph mode.
The default is `false`, preserving existing presets and text-wide behavior.

Exceeding counts cover all measured items, including findings omitted by the output limit. When the required input is unavailable the count is absent, rather than zero. Opt-in CLI telemetry includes only aggregate `length_checks.measured_gates` and `length_checks.exceedances`, with no content or gate IDs. Exceedances count gate-item pairs, so an item checked by two gates can count twice; unavailable gates are excluded.

### Repeated terms

The opt-in `repeated_terms` gate checks configured literal terms:

```toml
[[gates]]
id = "editorial.repetition"
kind = "repeated_terms"
terms = ["very", "really"]
max_occurrences = 2
whole_words = true
case_insensitive = true
blocking = false
```

Matching is non-overlapping and case-sensitive by default. The optional
case-insensitive mode lowercases each character; it does not perform full Unicode
case folding. Equivalent configured terms after case and whitespace normalization
are rejected in this mode to avoid duplicate findings. By default Unicode alphanumeric
characters and underscores define word boundaries; set `whole_words = false`
for substring matches. It does not infer synonyms, grammar, or editorial intent.
Each finding points to the first occurrence beyond the configured maximum,
and measurements count configured and exceeded terms and matched occurrences.

### Collection metadata uniqueness

`unique_collection_metadata` compares one metadata field across an explicitly
supplied collection. It reports duplicate items and their first matching item,
without echoing metadata values. Matching is case-sensitive and exact.

```toml
[[gates]]
id = "collection.unique_slug"
kind = "unique_collection_metadata"
field = "slug"
# Optional: compare values within each language rather than across languages.
group_by = "lang"
blocking = true
```

Rust hosts provide a complete collection in `ContentEvaluationContext` with `evaluate_with_capabilities`.
The ordinary single-item interface reports this gate as `not_checked`. Missing
or blank metadata in any item also yields `not_checked`; detected duplicates
remain visible as findings. When `group_by` is configured, values are compared
only within the same exact, case-sensitive group. A missing or blank group makes
the result `not_checked`; observed duplicates with known groups remain visible.
`items_missing_group` counts affected entries without logging group values. The
host must supply the group metadata consistently; Aexeo does not infer locale
fallbacks or normalize language tags. Omitting `group_by` retains collection-wide
matching. The host is responsible for collection completeness.
Rust EmDash hosts can use `content::evaluate_collection` to adapt a complete
slice of documents; document and site gates without their inputs remain
`not_checked`. JavaScript hosts can use `evaluateContentCollection` from the
EmDash content API. The CLI accepts a JSON array of content inputs with
`--collection items.json --no-text`, for example `[{"id":"/a","metadata":{"slug":"a"}}]`.
IDs must be unique and nonempty. If a collection is known to be partial, use
`--collection-incomplete`, `evaluate_with_collection_completeness(..., false, ...)` in Rust,
or `evaluateContentCollection(documents, preset, sources, { complete: false })`
in JavaScript. Observed duplicate findings remain visible, but uniqueness is
`not_checked`, even for an empty partial collection. The default assumes the
caller supplied a complete collection; Aexeo cannot detect silently omitted
records. Other gates that need an individual document still report `not_checked`. To check the
real WASM boundary locally, build the plugin and run `npm run test:wasm` in
`packages/aexeo-emdash`; the ordinary TypeScript unit test uses a mock.

### Markdown source locations

For Markdown CLI reports, finding `line` and `column` retain their existing
coordinates in extracted prose. The additive `source_locations` array supplies
original-file coordinates for exact, untransformed text spans, indexed by
`gate_id` and zero-based `finding_index`. An absent entry means no reliable
source position was found; escaped or transformed Markdown may be unmappable.
Unchanged text at the edges of a transformed text node can still be mapped;
the mapping does not claim positions inside encoded entities or escapes.
These locations apply to positional prose gates and to `link_destinations`,
`heading_length` and `duplicate_headings` findings, not metadata, site or
collection findings. Soft-wrapped text maps directly only where the wrapped
source is unchanged; a wrapped line inside a block quote has no location. Inline
code maps to its value inside the backticks. The map is local; opt-in telemetry includes only mapped and
unmapped counts and mapping duration, without positions or content.

### EmDash source locations and telemetry

The EmDash `evaluateContent` result includes `source_locations` for positional
prose, heading and invalid link destination findings. Link locations identify the
first marked span of each occurrence. Each entry identifies a `gate_id`, zero-based `finding_index`,
`block_index`, `span_index`, and `utf16_offset` within that span. Coordinates
refer only to the exact Portable Text snapshot evaluated; recompute after edits.
A missing entry means there is no usable location. Other finding types are not
mapped. Existing projected finding coordinates remain unchanged.

A host may supply the optional `onTelemetry` callback to `evaluateContent` to
receive aggregate `evaluation_ms`, `source_locations_mapped`, and, when measured,
`link_destinations` counts. Counts sum gate measurements, so multiple gates can
check the same occurrence. Duration covers
the complete JavaScript wrapper, including serialization and WASM initialization.
No text, paths or identifiers are included; Aexeo does not transmit the event.

### Offline link destination rules

A `link_destinations` gate checks syntax and configured URI schemes without
making network requests:

```toml
[[gates]]
id = "editorial.links"
kind = "link_destinations"
allowed_schemes = ["https", "mailto"]
allow_relative = true
blocking = false
```

The default for `allow_relative` is true; schemes must be unique lowercase URI
schemes. Empty, whitespace-containing, malformed and disallowed destinations
produce findings that quote the destination, up to 200 characters, and its scheme.
Scheme-relative URLs beginning with `//` are rejected because
their effective scheme depends on the page context. A pass does not establish reachability, safety, source support,
or content accuracy. Markdown direct and ASCII reference links, GFM literal URLs,
and used EmDash link marks provide destinations. A literal email address in
Markdown prose is not a link its author wrote: it is outside this gate and
`link_count`. `<me@example.org>`, `[mail](mailto:me@example.org)`, bare
`www.example.org` (which GFM renders with `http`) and bare `http://` URLs are
checked, because they are authored links or web navigation. If Markdown references cannot be resolved
conservatively, or an adapter lacks destinations, the gate is `not_checked`.
Markdown reports can provide the source location of an invalid link occurrence,
including a reference link usage; locations point to the beginning of the link,
not necessarily its destination definition. Counters report checked and invalid
destinations without recording addresses in telemetry. Image destinations and raw HTML are outside this gate's scope.

### Duplicate headings

Use `duplicate_headings` to flag repeated non-empty heading text within an item:

```toml
[[gates]]
id = "editorial.unique-headings"
kind = "duplicate_headings"
case_sensitive = false
blocking = false
```

Matching trims leading and trailing whitespace. It is case-sensitive by default;
when disabled, it uses Unicode lowercase conversion rather than locale-specific
case folding. Empty headings are ignored. Repeated headings can be intentional,
so tune this opt-in rule to the content type. An adapter without headings reports
`not_checked`. The report includes heading and duplicate counts; a finding uses
the heading ordinal and does not claim an exact source position.

## Heading length

The opt-in `heading_length` gate counts Unicode scalar values in each heading
after trimming outer whitespace. Configure a positive `max_chars`, for example:

```toml
[[gates]]
id = "heading.length"
kind = "heading_length"
max_chars = 80
blocking = false
```

It reports heading ordinal and count, the longest heading and the number over
the limit; a missing heading capability is `not_checked`. Unicode scalar values
are not grapheme clusters or display width, so combining marks and emoji can
count as multiple values. The limit is a project-specific heuristic, not a
universal SEO rule. Finding coordinates remain placeholders (1:1); Markdown
JSON reports include a separate source location at the start of the heading node.
EmDash reports map heading findings to the first non-empty span of the matching
heading block (offset zero), in the evaluated Portable Text snapshot.

### Paragraph length in characters

The opt-in `paragraph_character_length` gate limits each adapter-provided paragraph independently, useful when whitespace-separated word counts do not fit the content. Configure `max_chars` as a positive integer. It counts Unicode scalar values, including whitespace and punctuation; this is not a count of visual grapheme clusters or words. Measurements include `max_chars`, `paragraph_count`, `longest_paragraph_chars`, and `paragraphs_exceeding_limit`. Missing paragraph capability is `not_checked`.

```toml
[[gates]]
id = "editorial.paragraph_characters"
kind = "paragraph_character_length"
max_chars = 200
blocking = false
```

### Sentence length in characters

The opt-in `sentence_character_length` gate uses the same sentence-boundary heuristic, delimiters and optional abbreviation exceptions as `sentence_length`, but limits Unicode scalar values instead of whitespace-separated words. It excludes sentence delimiters and leading/trailing whitespace while counting internal whitespace and punctuation. It does not count visual grapheme clusters and is not linguistic sentence segmentation. Measurements include `max_chars`, `sentence_count`, `longest_sentence_chars`, and `sentences_exceeding_limit`; missing text is `not_checked`.

```toml
[[gates]]
id = "editorial.sentence_characters"
kind = "sentence_character_length"
max_chars = 100
delimiters = ".!?。！？"
blocking = false
```

### EmDash finding provenance

EmDash tags content-preset findings with `aexeo_origin: "content_preset"` so
a custom gate can reuse a legacy site-rule identifier without being filtered as
an unobserved rendered-page or site-graph finding. The configured plugin and
the version 2 sidecar response provide the tag. Legacy untagged stored findings
remain subject to conservative CMS filtering; re-evaluate records after upgrading
both the plugin and sidecar. Preset coverage logs add an aggregate
`colliding_findings` count, without rule IDs or content. The tag identifies the
evaluation path; it does not independently verify the source or the finding.

EmDash configured and version 2 sidecar evaluations attach available projected
block, span and UTF-16 offsets to each matching finding. These refer to the
exact adapted body snapshot, not necessarily a field or block in the original
CMS record: custom mappings and merged fields can change the structure. The
admin displays unavailable locations explicitly; older stored findings must be
re-evaluated to obtain them.

The `aexeo.preset_coverage` event also aggregates the number of reports with
mapping data and their eligible, mapped and unmapped finding counts. Older
bridges omit coverage; `mapping_reports` distinguishes that from zero findings.

For unchanged Portable Text blocks, EmDash additionally records the path to the
original text span in the evaluated CMS record and its UTF-16 offset. Paths are
snapshot-specific; re-evaluate after changes. Synthesized blocks and ambiguous,
cyclic or excessively large inputs do not receive a CMS source path. The local
`aexeo.cms_location_coverage` event reports eligible, mapped and unmapped counts
without paths, content or identifiers.
