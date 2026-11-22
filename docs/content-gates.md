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

By default the CLI reads the supplied UTF-8 bytes as raw text, including Markdown syntax. With `--input-format markdown`, it parses CommonMark with GFM extensions and projects text and headings while excluding front matter, code blocks and HTML; link labels, image alt text and inline code text are included, matching EmDash, which keeps the text of code-marked spans. Word counts split on whitespace and on known adapter block boundaries. Locations in raw-text mode refer to the supplied text. Markdown and EmDash findings refer to their extracted plain-text projection, not source line positions. When a profile requests lists and links together, the Markdown adapter derives both observations from the same parse; unavailable embedded HTML remains not checked. The existing opt-in adapter timing is aggregate and excludes content.

Markdown and EmDash adapters report where each heading, paragraph, list item and table cell ends. Those block ends terminate a sentence and a literal term cannot match across one. Soft-wrapped lines remain one sentence. Raw text has no known block structure.

Literal matching for `forbidden_terms`, `required_terms`, `repeated_terms` and the claim of an evidence gate ignores whitespace differences: `best price` matches `best` and `price` separated by any run of whitespace without a blank line. Finding positions stay in the original text. Quote anchors in evidence sources remain exact.

`forbidden_terms`, `required_terms` and `repeated_terms` accept `whole_words` (default `true`; set `false` for substring matches) and `case_insensitive` (default `false`). Case-insensitive matching lowercases each character, without full Unicode case folding. Word boundaries use Unicode alphanumeric and identifier-continuation characters, including underscores and many combining marks; no Unicode normalization is performed. Adapter block boundaries also separate words even when no separator is inserted; a term never matches across a known block boundary. Thai, Lao, Myanmar, Khmer, CJK and Hangul characters do not act as word characters, because these scripts do not reliably separate words with spaces; a term in them matches inside surrounding text.

### Review an explicit metadata date

`metadata_age` compares a supplied `YYYY-MM-DD` metadata value with an explicit
`as_of` calendar date. Configure both the date field and threshold deliberately:

```toml
version = 1
name = "review-age"
[[gates]]
id = "editorial.review_age"
kind = "metadata_age"
field = "last_reviewed"
as_of = "2024-03-01"
max_days = 365
```

The gate is advisory by default. It reports the age, threshold, and reference
date (`as_of_yyyymmdd`); missing, malformed, unavailable or future dates are
`not_checked`. The reference is fixed in the resolved preset, not read from the
clock: supply and refresh it for each evaluation or scheduled review. A recent
date does not prove that the article or its sources are accurate or current. A
CMS host must explicitly supply the chosen date in document metadata; an update
timestamp is not automatically a substantive review date.

### Character count for multilingual content

`character_count` bounds Unicode scalar values in the adapter-projected text,
including whitespace by default. Set `ignore_whitespace = true` to omit Unicode
whitespace. This can be useful for writing systems without space-separated
words; it does not count grapheme clusters, visual width or linguistic words.
Combining marks and emoji sequences can count as multiple scalars. Adapters may
insert or omit separators, so choose limits for the actual input format. Missing
text is `not_checked`, while observed empty text has count zero.

```toml
[[gates]]
id = "article.characters"
kind = "character_count"
min = 100
max = 2000
ignore_whitespace = true
blocking = false
```

At least one bound is required and `min` must not exceed `max`. The report
includes configured `min_characters` / `max_characters` and, when text was
observed, `character_count`. Existing word-count presets remain unchanged.

### Input limits

The CLI bounds auxiliary files separately: each custom preset, metadata file and evidence source is limited to 16 MiB, with a shared 64 MiB budget across those files and the collection JSON. A collection can consume up to the remaining shared budget. Exceeding an auxiliary limit stops the command with an error; it does not produce a partial report. With `--telemetry`, a local `content_auxiliary_input_limit` event records the input category and effective byte limit without paths or contents.

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

The opt-in `portable-text-link-review` preset detects explicit Markdown links
such as `[source](https://example.org)` embedded in observed Portable Text.
It is advisory: verify the host rendering before changing content or links.
It ignores code-marked spans and Markdown code, and can map a finding to its
original span. It requires complete text projection; unsupported adapters or
incomplete projection report `not_checked`. It does not validate link targets,
rendered HTML or bare citation markers. Explicit two-bracket reference syntax is
reported as a candidate even when a definition is unavailable; adjacent bracketed
text can also match, so editors must review the context. Unfinished inline
HTTP(S), root-relative, dot-relative and fragment link syntax is also reported
when recognizable in source text; this narrow heuristic can miss other malformed
links and does not validate URLs.
Compose it with another preset using `extends`, and enable it only where the
host renders Portable Text as rich text rather than interpreting Markdown.

Numeric reference labels linked to `#reference-…` are masked in sentence-rhythm
projections to avoid merging sentences. Literal required, forbidden and repeated
term checks in the content CLI, change verdict, incremental evaluation and EmDash
instead see those labels in Markdown, HTML and Portable Text, while keeping the
original projected offsets. Rust hosts using the adapters directly must pass the
optional literal projection to `evaluate_with_literal_projection`. Other checks still use their documented
projection; this does not verify citations or source support. If a supplied
literal projection is inconsistent with the observed text, the literal and
external checks are `not_checked`. External extensions receive the restored
reference labels in their projected text, and results bound to an earlier label
are stale when that label changes. EmDash also preserves labels in projected
headings when headings are available independently of body text. Previously captured results that used masked
labels must be regenerated. This binds the projection, not the full rendered
page or the correctness of the extension's judgment.

The opt-in `fragment-readable` preset is intended for an individual introduction, example, prerequisite, or other fragment. It composes only the advisory shared sentence-length check; it does not require article length or page metadata. Select it explicitly for fragments, and configure other checks according to the field and language. A clean result does not establish completeness or factual accuracy.

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
`max_occurrences`. Matching lowercases, trims surrounding punctuation and
compares canonically equivalent Unicode spellings while preserving accents; it
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
more than 10,000 distinct openings, more than 4 MiB of normalized distinct
opening keys, or more than 100,000 paragraphs stops the check as `not_checked`,
retaining findings from the portion already assessed. Measurements report the
total paragraph count and the number actually evaluated, including partial runs.

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

Required-heading measurements separate configured, present and missing headings
when headings are available. Required-metadata measurements separate configured,
present, missing and unavailable fields when metadata is supplied. A missing
count describes only observed fields; fields the adapter cannot observe count as
unavailable and make the gate `not_checked`. When the entire input is unavailable,
observed counts are omitted rather than reported as zero.

Save this as `product.toml`, then run:

```sh
aexeo-cli content article.md --preset product --preset-source product=product.toml --metadata metadata.json --format json
```

To bound the length of one metadata value, compose a `metadata_length` gate:

```toml
[[gates]]
id = "metadata.description_length"
kind = "metadata_length"
field = "description"
min = 40
max = 160
blocking = false
```

Bounds count Unicode scalar values, including whitespace; they are not visual
width or grapheme counts. At least one bound is required. An absent field and an
unavailable field both leave the length `not_checked`, with distinct reasons;
an explicitly empty value has length zero. Compose `required_metadata` when
presence must also be enforced. Findings are not assigned a position in the
article body. Length does not establish SEO performance or editorial quality.

Heading names are matched exactly and case-sensitively. Raw-text input has no heading capability, so heading gates are `not_checked`; Markdown and EmDash provide it.

Metadata must be a JSON object with string values. Without `--metadata`, metadata gates are `not_checked`, rather than treating missing input as an empty set of fields. An empty object indicates that metadata was supplied but contains no fields. Adapters can also mark individual metadata fields unavailable. A required-metadata gate then remains `not_checked`, while findings for other known-missing fields remain visible. Its `unavailable_fields` measurement counts unknown required fields; `present_fields` counts known, nonempty fields. EmDash treats an unobserved description as unavailable, while an explicitly observed empty description is missing.

`extends` is a list of named presets. Parents are applied left to right; gates with an existing ID replace the earlier definition in place, and new gates append. `disable = ["gate.id"]` removes inherited gates and rejects unknown IDs. Supply each custom parent with a separate `--preset-source name=path`; built-ins are already available. Cycles, unknown parents, duplicate local IDs, unknown fields, invalid bounds and unsupported versions fail resolution. Inheritance paths are limited to 64 presets, including the selected preset. Shared parents are reused during each resolution; changing a source takes effect on the next resolution. A preset's `name` is its report label, while its source name is the key used in `extends` and `--preset`.

Use `--explain --explain-origins` to inspect the resolved preset alongside the
winning source name for each active gate, in gate order; add `--format json`
for structured output. This separate explanation envelope is not a reusable
preset file. A source name is the caller-supplied key, not proof of authorship
or an authenticated path. Disabled gates are absent. If a custom source uses
the same key as a built-in, it replaces that built-in before resolution. If the
CLI receives the same source key more than once, the last supplied source wins.
The existing `--explain` output remains a standalone resolved preset. Optional
`--telemetry` emits only the number of active gates and whether origins were
requested, without preset names, paths or rule parameters.


Each gate may set `severity` to `error` or `warning` (default), and `blocking` to true (default) or false. Status always reports the gate result; the CLI exits 1 for a failed blocking gate, 2 if a blocking gate was not checked (even if another blocking gate failed), and 0 otherwise. Non-blocking gates still appear in the report, including when they could not be checked. Invalid configuration, invalid presets and unreadable inputs exit 2; 70 is reserved for internal errors in Aexeo. Content is unchanged unless `--fix` is passed explicitly.

The EmDash package exposes `evaluateContent` from `@aeptus/aexeo-emdash/content`; the Rust bridge and WASM API accept the same named source map and preset. A missing EmDash body is unavailable for text, heading, paragraph and link gates; an explicitly supplied empty body is evaluated as empty. The document adapter has no site capability, so site gates are `not_checked`. Configured-mode plugins can opt into a document preset for both save hooks and refreshes:

```ts
aexeoPlugin({
  contentPreset: { name: "article", byCollection: { glossary: "glossary" } },
})
```

For custom presets, add `sources: { custom: "...TOML..." }` and select `name: "custom"`. `name` is the fallback preset; `byCollection` selects a named preset for a specific collection on both save and refresh. The existing SEO findings remain enabled. A gate the document adapter cannot check is recorded as `not_checked` with its reason. For the sandboxed plugin, configure equivalent presets on the [sidecar worker](../packages/aexeo-crawl-worker/README.md) using its environment variables.

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
`--link-timeout` (1–30 seconds, default 5) bound requests. Without either an
observer flag or an observation snapshot, or when the adapter cannot supply
complete destinations, the gate is `not_checked`.
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

### CLI host observations

A trusted host can instead supply `--link-observations observations.json` for a
single Markdown, HTML or adapter JSON document, without network access. Generate a template
with `aexeo-cli content article.md --input-format markdown --export-link-observations`;
it contains the digest and extracted destinations with every observation set to
`not_checked`. The output can contain private URLs. A trusted host may fill in
its observations and supply the resulting snapshot to the content command.
The strict UTF-8 JSON snapshot has this shape:

```json
{
  "version": 1,
  "source_sha256": "<lowercase SHA-256 of the exact source bytes>",
  "destinations": ["https://example.org/a", "https://example.org/b"],
  "observations": ["not_found", "not_checked"]
}
```

The source digest, destination values and occurrence order, and observation
count must all match the document currently evaluated. For adapter JSON the
digest covers the exact supplied JSON bytes, including structure and source maps,
not just projected text. Regenerate snapshots made by earlier versions for this
format; formatting changes also require a new snapshot. Duplicate and unknown JSON
fields are rejected. Valid observations are `reachable`, `not_found`,
`not_checked`, and `not_applicable`. A mismatch is an error; incomplete
observations retain any observed findings but leave the gate `not_checked`.
The snapshot is mutually exclusive with live link checks and `--fix`. It is
limited to 16 MiB and counts against the shared auxiliary input limit.

The host must supply trustworthy, current observations and decide which links
are applicable; matching the source does not establish freshness or accuracy.
Snapshot files may contain private URLs. Optional telemetry emits only aggregate
counts and elapsed time, with no source, paths or destinations.

### Change-wide host observations

A `content-change` host may pass `--link-observations change-links.json` to
supply observations without network access. Generate the complete template in one
offline operation, using the same HTML scope as the subsequent evaluation:

```sh
CONTENT_GATE_CHANGE_FILE=change.json CONTENT_GATE_VERDICT_FILE=change-links.json \
  aexeo-cli content-change --export-link-observations --html-scope main --telemetry
```

No preset or network access is needed. The command validates source digests and
rejects the entire export if any post-change entry cannot be projected; a prior
output remains intact on failure. All observations start as `not_checked`; a
trusted host can replace them with its own results before importing the file.
Telemetry contains aggregate counts and elapsed time only. The template may
contain private URLs. Individual `content --export-link-observations` snapshots
can also be assembled under their change entry identifiers:

```json
{
  "version": 1,
  "change_id": "example",
  "entries": {
    "article": {
      "version": 1,
      "source_sha256": "<SHA-256 of the exact entry bytes>",
      "destinations": ["https://example.org/a"],
      "observations": ["not_checked"]
    }
  }
}
```

Each supplied entry must refer to an `after` state with the same digest and
its extracted destinations in the same order. Export using the same HTML
scope as the change evaluation. Links in missing entries and inconclusive
observations remain `not_checked`; a supplied entry that cannot be assessed, a mismatch,
duplicate keys or unknown fields rejects the run before replacing the verdict.
The snapshot is limited to 64 MiB and must use a path distinct from the output.
It cannot be combined with live checks or offline receipts. It can be combined
with external gate results. During `--verify-verdict`, Aexeo compares a digest
of the complete supplied snapshot before evaluating and checks the observations
used in the fresh verdict. The host must establish the trust and freshness of
observations and prevent concurrent input changes. Snapshots contain URLs and
may be private; optional `content_change_link_snapshots` telemetry contains only
aggregate supplied and missing counts and elapsed time.

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

For automatic current-field audits, install `fragmentLinkObservations` separately
in the runtime and configure `autoAuditFragments` and the field presets. The
callback receives the field name as well as the collection, route, locale, preset,
and a copy of the fragment's ordered destinations. It runs only for fragment
presets requiring reachability; article and historical revision observers are
independent. Missing projections and invalid or failed responses remain not checked.

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

## Host adapter JSON

A host with a different content format can supply a versioned projection to the
single-content CLI: `aexeo-cli content projection.json --input-format adapter-json
--preset article --format json`. The JSON object uses `schema` equal to
`aexeo.content-adapter/v1` and `text` containing projected plain text or null.
Optional fields are `headings` (strings), `heading_levels` (1–6, one per heading),
`paragraphs` (ordered nonoverlapping UTF-8 byte ranges `{start,end,list_item?}`
in the projected text), `block_ends` (increasing UTF-8 byte offsets), `lists`
(`{item_count,depth}` for each observed list), and `link_destinations` (strings
in occurrence order). Paragraph `end` is exclusive. Preserve separation between
projected blocks in `text`, or provide `block_ends` at their boundaries so words
are not joined. Example, including an optional source map:

```json
{"schema":"aexeo.content-adapter/v1","text":"Hello world.",
 "headings":["Introduction"],"heading_levels":[1],
 "paragraphs":[{"start":0,"end":12,"list_item":false}],
 "block_ends":[12],"lists":[],"link_destinations":[],
 "source_map":{"source_text":"Introduction\nHello world.",
               "spans":[{"projected_start":0,"projected_end":12,"source_start":13}],
               "heading_offsets":[0]}}
```

Each supplied observation array is the host's assertion that the observation is
complete; an empty array is observed empty, while an omitted or null array is
unavailable. The optional source-map arrays can describe partial locations.
Paragraph-list classification is unavailable unless every paragraph has an
explicit `list_item`. Invalid byte ranges, boundaries, heading levels, duplicate
JSON keys and unknown fields are rejected. The input is bounded by the CLI's
content byte limit and observation counts are bounded. Metadata can be supplied
separately with `--metadata` as for other formats. The host is responsible for
faithful extraction, completeness and snapshot provenance; Aexeo cannot infer
them from JSON. Optionally, `source_map` contains `source_text` and `spans` of
`{projected_start, projected_end, source_start}`. Offsets are UTF-8 bytes;
projected ends are exclusive. Both projected and source ranges must be ordered,
non-overlapping and byte-identical; invalid ranges reject the input. The map may
cover only part of the projected text. Prose findings in these spans can receive
separate positions in the supplied `source_text`. An optional `heading_offsets`
array supplies one UTF-8 byte offset or null per heading, in heading order.
Non-null offsets must identify nonempty, byte-identical heading text in ordered,
nonoverlapping source ranges. Heading-length, duplicate-heading and hierarchy
findings can then receive positions for those headings; null entries stay
unmapped. An optional `link_offsets` array similarly supplies one UTF-8 byte offset or null per link destination, in occurrence order. Non-null offsets must match the nonempty destination bytes in ordered, nonoverlapping source ranges. Invalid-destination and externally observed unreachable-link findings can then receive positions; null entries remain unmapped. Encoded or otherwise different source bytes must be left null. Optional `list_anchors` supplies one null or `{source_start, text}` per observed list, including nested lists, in occurrence order. Non-null anchors must be nonempty, ordered, nonoverlapping and byte-identical to `source_text` at the UTF-8 byte offset `source_start`. List-count findings then receive positions for mapped lists. The host must select an appropriate anchor within each corresponding list; the byte check does not establish list membership. The report labels these
positions `host_supplied_source_text`: matching bytes cannot
prove that the host selected the correct occurrence, actual source or snapshot.
The host must establish that correspondence and provenance. Finding coordinates may refer to projected text
or be placeholders, never to the JSON file. `--fix` and `--export-adapter-input` are unavailable
for this format. Reports bind the original JSON bytes, including its observations, in their input fingerprint.
Feedback capture, corpus export and replay accept this format with the exact JSON
as the example text. Corpus examples must contain a string `text` projection;
exact and normalized cross-split checks compare that projected text, even if
other observations differ. These checks cannot establish independent sources.
Opt-in telemetry records aggregate observation counts and adapter duration, not
text, identifiers or destinations. This is not a publication authorization.

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

### Preparing a blind review packet

A reviewer can assess examples without receiving the proposed labels, the split,
or Aexeo's decisions. Supply a clear rubric, create a private packet, and give it
only to an authorized reviewer:

```sh
python3 scripts/content-review-note.py blind-packet --corpus corpus.json \
  --split holdout --rubric-file rubric.txt --output review-packet.json --telemetry
```

For each reviewed example, the reviewer fills in `human_status` with `passed`,
`failed`, or `uncertain`; leaving it `null` skips it. Then import those explicit
opinions into a new private corpus copy:

```sh
python3 scripts/content-review-note.py bind-blind-reviews --corpus corpus.json \
  --packet review-packet.json --reviewer reviewer-id --method 'independent manual' \
  --output reviewed-corpus.json --telemetry
```

If a pass/fail differs from the proposed label, the entire import fails unless
`--accept-human-label` is explicitly provided; the former label is retained in
`previous_expected`. Changed text or metadata, unknown or duplicate IDs and
already-reviewed examples are rejected. Uncertain and unanswered examples remain
unreviewed; retain the packet to preserve those responses. At least one pass or
fail is needed to create a reviewed corpus copy. The review records the text,
metadata and supplied rubric hashes, reviewer and method, but no model decision.
It does not prove reviewer identity, independence, what material they actually
saw, or that the rubric corresponds to a particular policy. The packet can itself
reveal information through the supplied rubric, identifiers or content; it
contains full text and metadata and should be handled as sensitive. Never use
synthetic regression fixtures as independent human evidence. Output files are
created with private permissions and cannot overwrite existing files. Optional
local telemetry contains only aggregate counts and duration.

### Combining blind reviews from several people

Give separate copies of the same packet to reviewers. Once they have recorded
opinions, combine the copies in matching packet, reviewer and method order:

```sh
python3 scripts/content-review-note.py combine-blind-reviews --corpus corpus.json \
  --packet review-a.json --reviewer reviewer-a --method manual \
  --packet review-b.json --reviewer reviewer-b --method manual \
  --output blind-reviews.json --telemetry
python3 scripts/evaluate-content-gates.py corpus.json --binary target/release/aexeo-cli \
  --split holdout --review-events blind-reviews.json
```

This produces a private version 4 review log. It preserves each reported
pass, fail or uncertain response, the reviewer and method, and hashes of the
text, metadata and rubric; it contains no model decision or full text. Repeated
example/reviewer pairs, stale inputs and different rubrics for the same example
are rejected. Retain the packets when the full rubric or unanswered entries are
needed. The command creates a new file with private permissions and refuses to
overwrite one. Optional telemetry contains only aggregate review count and time.

The corpus report counts examples with multiple reviews, pass/fail disagreement,
and uncertain opinions. Since these reviews contain no model decision, its
`counts` and `override_rate` are null and `model_comparison_available` is false.
The report does not pick a winning label or change the corpus: adjudication is a
separate human decision. Reported identities and matching rubric hashes do not
prove independence or that reviewers saw the same material. Older evaluators may
reject version 4 logs; keep those logs private because they contain reviewer and
example identifiers and input hashes.

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

### Detecting normalized cross-split matches

Each evaluation reports `normalized_split_overlap` across the entire corpus,
not only the selected split. It counts groups and examples on each side whose
text matches after NFKC Unicode normalization, Unicode case folding and
collapsing whitespace; it contains neither text nor identifiers. Exact
cross-split text duplicates remain rejected by corpus validation. To reject
normalized matches before invoking the evaluator as well, use:

```sh
python3 scripts/evaluate-content-gates.py corpus.json --binary target/release/aexeo-cli \
  --split holdout --reject-normalized-overlap
```

To identify the examples for a private review, without invoking Aexeo, run:

```sh
python3 scripts/evaluate-content-gates.py corpus.json --list-normalized-overlaps
```

This explicit inspection prints a JSON `groups` array of development and holdout
example IDs, plus the same aggregate counts. It does not print corpus text,
metadata or hashes; IDs may nevertheless be sensitive. Protect the terminal,
logs and any redirected output. The command validates the corpus and exits 0
when successful. Adding `--reject-normalized-overlap` still prints the groups,
then exits 1 when overlaps exist; invalid inputs exit 2. It does not change
corpus labels or move examples. The normal evaluation report remains aggregate-only.

This is a conservative curation signal: case or Unicode differences can be
meaningful, and this check ignores metadata. Review matches before changing
splits or labels. Unicode behavior depends on the Python runtime's Unicode
database. A zero count does not rule out paraphrases, partial excerpts,
incorrect source groups, review leakage or other shared sources; it does not
prove independence. The counts are local report telemetry and are not sent
anywhere by the script.

## Local latency measurement

For local content and crawl evaluation, `--telemetry` also emits `gate_timings_ms`, an
array in resolved preset order. Values measure the execution of each gate,
including result construction, and omit preset validation, source mapping and
output rendering. The event contains no gate IDs or content. Instrumentation
adds clock and observer overhead; timings are diagnostic rather than a benchmark.
For crawl evaluation, gate and evaluation timings exclude crawling itself.
For local content, `stages.input_load_ms` covers input loading and `stages.adapter_ms` covers adaptation; neither includes external-result loading or link observation. When an external results file is supplied, `stages.external_results_load_ms` measures its loading and validation. The separately emitted `content_link_reachability` event measures opted-in link observation. These durations are local diagnostics and contain no paths or URLs.

Run `cargo run --release --locked --offline -p aexeo-core --example content_bench` to measure the content gates alone and Markdown parsing plus evaluation over a synthetic 29 KB document. It reports p50 and p95 across 100 iterations; figures depend on hardware and concurrent load and exclude CLI process startup. With `--json` it also emits machine-readable p50, p95 and average measurements for individual evaluation, Markdown parsing plus evaluation, batches of 50, and HTML projection with and without evaluation. The HTML fixture contains 500 paragraphs and links; its evaluation covers word count, forbidden terms, required headings and link destinations. `scripts/check-performance.sh` enforces generous average-time and p95 regression ceilings; these are not latency guarantees for other workloads. The fresh-process CLI benchmark includes startup and evaluation of a tiny text input, but does not flush operating-system caches and is not a cold-cache measurement.

For a synthetic preset with twelve case-insensitive literal gates, run `cargo run --release --locked -p aexeo-core --example literal_gate_bench`. It emits aggregate input size, iteration count and p50/p95 timings locally; `-- --report` also prints the synthetic report for output comparison. CI enforces an initial average and p95 budget for this fixture. The limits include substantial headroom over a single GitHub runner observation (1.011 ms p50, 1.046 ms p95); they are provisional and do not guarantee performance for other workloads. Literal gates reuse at most one prepared case mode during an evaluation, never across content inputs; alternating modes rebuild it.

## Explicit safe fix

The `final_newline` gate requires the original source text and reports a non-empty file without a final newline. Add `[[gates]]`, an `id`, and `kind = "final_newline"` to a preset to enable it. Run `aexeo-cli content article.md --preset custom --preset-source custom=rules.toml --fix` to append a newline when needed. The fix preserves existing CRLF line endings, reports its gate ID in `applied_fixes`, and is idempotent. It requires a local regular file, rejects symlinks and, on Unix, files with multiple hard links, and compares the open file with the version read before appending. The candidate is evaluated in memory before the file is changed; loading or evaluation errors leave it untouched. Opt-in telemetry reports the number of applied fixes without content or identifiers. It does not change prose. The EmDash adapter has no original file source, so this gate is `not_checked` there.

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

Supply the source using the same evidence snapshot interfaces described above. A missing source, invalid JSON, duplicate object keys at any depth or missing pointer is `not_checked`; a different value or absent claim is a failure. Comparison is typed JSON equality, not numerical inference. The preset author is responsible for mapping the claim to the correct field and value; the engine does not understand the claim, assess source reliability, or verify reality. Version and review both the preset and source.

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

Supply both named snapshots, for example `--evidence-source study=source.txt --evidence-source study-review=review.json`. `support` accepts `supported`, `contradicted` or `uncertain`; `reliability` accepts `reliable`, `unreliable` or `uncertain`. Only supported plus reliable passes. Contradiction or unreliable source fails; uncertainty, missing inputs, malformed records, duplicate object keys at any depth and reviews of a different claim or source yield `not_checked`. The reviewer identifier must be nonempty. An optional `provenance` object can hold string metadata; it is informational and does not authenticate the review. A changed source requires a new review.

### Source review age

Compose `evidence_review_age` with `evidence_review` when reviews must be
revisited periodically. The caller supplies a fixed reference date and the
review record contains a `provenance.reviewed_at` calendar date:

```toml
[[gates]]
id = "study.review_age"
kind = "evidence_review_age"
source = "study"
review = "study-review"
claim = "The count is 14."
as_of = "2026-11-19"
max_days = 180
blocking = false
```

For example, add `"provenance":{"reviewed_at":"2026-09-01"}` to the review
record. A review older than `max_days` produces an advisory finding by default.
The claim must occur in observed text and the review must match the exact claim,
source and a nonempty reviewer. A missing, invalid or future review date, an
unavailable source or content, or an unmatched review is `not_checked`. Both
`as_of` and `reviewed_at` use strict `YYYY-MM-DD` dates; update `as_of` for each
run according to the host's policy. Reports contain aggregate age and threshold
measurements when available. Set `blocking = true` only when your workflow
intentionally requires a current review; unchecked blocking gates remain unchecked.

This gate checks the supplied review date only. It does not assess source
reliability, support, authorship, factual accuracy or whether a review was
actually performed. Compose it with the source review gate and use trusted,
independently protected records; the review date is a caller assertion.

The [source review process](source-review-process.md) describes how an application can record a claim-specific human assessment and when to reassess it.

The review itself is **trusted input**: Aexeo does not authenticate the reviewer, run an NLI model, calibrate its confidence, or independently verify the stated judgments. Applications must choose and evaluate their reviewer, protect the assessment's provenance, and decide when a human should review it. Exact text binding prevents accidental reuse against different input, but does not prevent a forged review.

An optional [Jev adapter](../adapters/jev/README.md) produces this review format from an explicit remote model call and a separately supplied source-reliability judgment. It is not part of the default offline engine.

### Paragraph count

### Required literal terms

`required_terms` checks that every configured term occurs in the supplied text.
It accepts 1–256 nonempty, distinct literals, `whole_words` (default `true`),
`case_insensitive` (default `false`), `severity` and `blocking`. Missing text is
`not_checked`; observed empty text fails. It stops searching each term at its
first match and reports aggregate `configured_terms`, `present_terms` and
`missing_terms`. A missing term is a whole-document finding, not a source
location. The check establishes literal presence only, not meaning, coverage,
accuracy, relevance or that a term is used appropriately. Existing local per-gate
telemetry applies; it contains no terms or content.

```toml
[[gates]]
id = "article.required_terms"
kind = "required_terms"
terms = ["incident response", "support"]
case_insensitive = true
blocking = false
severity = "warning"
```

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

`delimiters` lists the characters that end a sentence and defaults to `.!?`. A decimal point between ASCII digits does not end one, and neither do Markdown hard breaks. Explicit lowercase `http://` and `https://` URLs of at most 2,048 bytes, recognized within a single adapter block, keep internal delimiters inside the sentence; trailing punctuation is treated as prose punctuation. Invalid, longer, or other URL forms retain the normal delimiter behavior. This is a heuristic and does not check links or their content. Closing brackets and typographic closing quotes immediately after a delimiter (including after spaces) do not start a new sentence; straight quotes are treated as closing only when adjacent. Tokens made only of quotation marks or brackets do not count as words in this gate. Adapter block ends always do, so unpunctuated bullets, headings and table cells are measured separately. This is a heuristic, not linguistic segmentation. By default abbreviations end a sentence. Optionally set `abbreviations = ["Dr.", "M."]` to keep the period of an exact, case-sensitive, whitespace-delimited abbreviation inside a sentence. This also applies at a true sentence end, so choose exceptions for the content carefully; adapter block boundaries still end sentences. Multi-period abbreviations such as `e.g.` and `U.S.` are supported when the entire configured token is observed within one block; trailing comma or other closing punctuation is allowed. These tokens are limited to 128 UTF-8 bytes and 16 periods. Words attached to opening punctuation are not recognized by this option. Languages without whitespace-separated words need tailored rules. The optional `require_latin_word_boundaries = true` conservatively skips sentences outside Aexeo’s recognized Latin character subset and marks the gate `not_checked`, retaining any findings from assessed sentences. It reports `sentences_with_unverified_word_boundaries`; this is a character-set heuristic, not language detection or proof that the remaining segmentation is correct, and it may also skip sentences containing emoji or other symbols. Custom gates default to false for compatibility; the built-in `readability` preset enables it. Measurements report the limit, assessed sentence count, longest assessed sentence and `sentences_exceeding_limit`; absent text is `not_checked`.

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
characters, identifier-continuation marks and underscores define word boundaries; set `whole_words = false`
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
matching. The host is responsible for collection completeness. To bound work
across adapters, the gate examines at most 100,000 items, 10,000 distinct
(group, value) pairs and 4 MiB of distinct key bytes; each group or value is
limited to 16 KiB. If a limit is reached, the result is `not_checked` and
retains findings from the examined prefix. `collection_items` counts all
supplied items, while `items_evaluated`, missing-field counts and duplicate
counts describe only that prefix. Direct Rust evaluations reject blank or repeated
item IDs encountered in the evaluated prefix, instead of producing ambiguous
findings. CLI and EmDash validate the supplied collection identities at their
boundaries.
For `content-change`, a host can explicitly supply a post-change collection
with `--collection-snapshot snapshot.json`. Its JSON shape is
`{"change_id":"example","complete":false,"items":[{"id":"article","metadata":{"slug":"article"}}]}`.
The change ID must match, item IDs must be unique, proposed item metadata must
match the change, and deleted or old renamed IDs must be absent. The v1 verdict
subject does not bind the snapshot or entry metadata: a runner must retain and
compare these and other evaluation inputs independently before applying a verdict.
The snapshot
must represent one common collection scope. The host asserts completeness and
revision consistency; Aexeo cannot detect omitted unchanged records. A partial
snapshot preserves observed duplicates while keeping the check `not_checked`.
Experimental candidate-bound verdicts also accept this snapshot. Its identity
set must equal the overlaid candidate's when complete, or be a subset when
incomplete. Changed non-delete entries must be present. The collection bytes are bound into
the verdict and checked on revalidation. The host still must establish metadata
provenance and completeness; matching identities do not verify metadata values.

The collection gate runs once for the change, including delete-only changes;
site impact of deletion and rename remains separately `not_checked`. Optional
telemetry reports only item count and the supplied completeness flag. Rust hosts
can use `evaluate_change_with_collection` and `ChangeCollection`.

The verdict output path must differ from every input file, including preset
sources, external results, collection snapshots, previous verdicts and confined
content files. Aexeo rejects direct paths, resolvable symlink aliases and, on
Unix, hard-link aliases. Hosts must prevent concurrent replacement of these paths
while evaluation runs; the alias check is not a filesystem transaction. Always
use a dedicated output path and check the exit code before consuming it.

To revalidate a previous offline verdict, pass `--verify-verdict previous.json`
to the same `content-change` invocation, with its current change, preset,
collection, options, and a **different** `CONTENT_GATE_VERDICT_FILE` output path.
Aexeo verifies the optional offline input fingerprint, reads and checks the
current content bytes, reruns the gates, and compares the full result except for
the evaluation timestamp. A mismatch, missing fingerprint, or unreadable input
returns exit code 2 and does not write a new verdict. On a match, it writes the
fresh verdict and returns its usual status code (0, 1, or 2). Always check the
exit code; an output path may contain an older file from a previous invocation.
Opt-in `--telemetry` reports `content_change_revalidation` with a boolean match
result, without the content, paths, or identifiers.
With `--check-link-reachability`, the CLI first checks the previous input
fingerprint and the configured `--link-max-checks` and `--link-timeout` before
making observations. It then observes links again and compares the complete
result and observation digest. A changed observation, including a newly unavailable one, causes a mismatch;
a matching result does not establish long-term availability or source reliability.
The previous verdict must have been created by a CLI version that records these
link options.

With `--extension-results`, revalidation checks a fingerprint of the entire
validated result snapshot before observing links, then compares a fresh verdict
and the external callbacks actually consumed. Both options may be used together.
The host must provide the current, trusted result snapshot: Aexeo does not rerun
or authenticate its producer and cannot establish whether its judgment is correct
or current. Verdicts generated before these bindings were added cannot be
revalidated in this mode and require a new evaluation.
Neither mode authenticates the previous timestamp, verifies collection completeness
or revision, or replaces the host's authority and freshness checks. Aexeo never
publishes content.

Rust hosts using a link observer without custom gate extensions can compare a
previous result with a newly evaluated verdict using `verify_observed_link_verdict`.
The `aexeo.observed-link-binding.v1` extension hashes the same change, policy,
limits, options and collection as the offline binding, plus the ordered observer
requests and responses actually used. The verdict contains only hashes; the host
must protect them as potentially guessable data. The caller must independently run
a fresh evaluation with trusted inputs and observer, then compare before applying
an action. This does not authenticate the observer, verify response freshness,
prove collection completeness or bind hidden external state. A separate
`aexeo.external-observations.v1` binding records a digest of custom callback
requests and responses, and optionally the link-observation digest. Rust hosts
can use `verify_external_input_binding` before external work and
`verify_external_verdict` against a freshly evaluated verdict. The CLI also
binds its entire supplied result snapshot. These checks do not authenticate
producers or prove the freshness or correctness of their results. Older
consumers that ignore the extension are not protected.
The observation hash is SHA-256 initialized with the UTF-8 extension identifier;
each actual callback adds an eight-byte big-endian length followed by compact
UTF-8 JSON `[entity_id, destinations, observations]`, in callback order. Each
observation uses its snake-case enum name. The input digest uses the existing
engine-specific offline binding, so this is not yet a general interoperable
contract for independent implementations.

Rust EmDash hosts can use `content::evaluate_collection` to adapt a complete
slice of documents; document and site gates without their inputs remain
`not_checked`. Explicitly unavailable metadata fields are excluded from
collection comparisons. JavaScript hosts can use `evaluateContentCollection`
from the EmDash content API. When `observed_metadata_fields` is supplied,
collection values and grouping keys outside that list are unavailable; known
duplicates remain visible, but incomplete metadata prevents a complete pass.
Omitting the list retains the legacy assumption that fields are observed. The CLI accepts a JSON array of content inputs with
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

### Static HTML content

Use `--input-format html` to evaluate an HTML document or fragment with content
presets. The adapter extracts text, headings, `<p>` paragraphs and `<a href>`
destinations from the supplied markup. It excludes `<head>`, script, style,
template, noscript and elements with the HTML `hidden` attribute. Images
contribute their `alt` text; image URLs are not link destinations. This is a
static markup projection, not a browser rendering: CSS, JavaScript, closed
`details`, navigation and generated content can affect what readers see. Scope
therefore depends on the fragment or page the host supplies. HTML parsing does
not itself preserve source offsets, so findings refer to projected text. Exact
source locations are supplied only when the conservative reconstruction below
can establish them. Network checks remain explicit opt-in.
Inputs exceeding 16 MiB or 100,000 traversed DOM nodes are `not_checked`, not empty.

For a full HTML page, `aexeo content article.html --input-format html
--html-scope main --preset article --format json` explicitly restricts the
content projection to the single non-hidden `<main>` element. This excludes
navigation and footers outside it. If none or more than one is found, dependent
gates are `not_checked`; an explicitly empty `<main>` remains an observed empty
body. The default `document` scope preserves the original full-document
behavior. The JSON report records `content_projection`, and opt-in telemetry
records the scope without content. Source locations are supplied for this
selection only when the entire document has a supported, well-nested lexical
structure and the selected reconstruction matches the parsed projection;
otherwise they remain unavailable. Navigation and footer occurrences are not
mapped as part of the selected main element. Source-level checks and fixes apply to
the supplied file, not only the selected element. Scope does not prove CSS or
ARIA visibility, page identity, render completeness, or correctness of CMS
metadata. The input fingerprint binds the supplied bytes and metadata, not the
projection scope. Feedback capture retains this explicit projection in the
private note. To replay it in a corpus, set `"input_format": "html"` and
`"content_projection": {"format": "html", "scope": "main"}` at corpus level.
Evaluation, feedback export, bound reviews and blind review packets require the
same projection; mismatches are rejected. Reviewers must assess the selected
main element, even though packets contain the supplied HTML. Retaining the
report and scope does not authenticate the input, reviewer or review conditions.
For changes, `content-change --html-scope main` selects the same projection for
all `text/html` entries. The default remains `document`. Missing or ambiguous
main elements are `not_checked`. The option also applies to experimental offline
receipts; revalidation requires the same scope. `--telemetry` emits the selected
scope and eligible entry count without content, paths or URLs. This does not
change the content-gate v1 protocol.

Rust hosts may also call `evaluate_change_with_collection_and_html_scope` with `HtmlContentScope::Main` to evaluate only the unique observable `<main>` in each HTML change entry. Missing or ambiguous main elements remain `not_checked`; non-HTML entries retain their normal adapters. The verdict declares the scope in `aexeo.html-content-scope.v1` and binds it into its Aexeo-specific input fingerprint. `verify_scoped_change_input_binding` checks the intended scope before host callbacks; `verify_scoped_change_verdict` compares a fresh trusted result. Existing document-scope verification rejects main-scope bindings, and the existing API remains document-scoped. These helpers do not authenticate the evaluator or establish source freshness or publication safety. Static markup does not establish browser visibility.

The content-change protocol supports `text/html` with the same rules. For the single-content CLI, opt-in telemetry includes `html_projection`: a complete projection reports aggregate heading, paragraph and link counts; an unavailable projection reports its status without counts. The JSON report and telemetry distinguish unavailable source locations, byte-identical text (`identity_projection`), and a verified, well-nested literal HTML subset (`literal_html`). Prose locations in the latter are mapped only when a bounded lexical reconstruction matches the parser projection exactly. In the supported subset, heading and link-destination findings also map to their opening tags; link reachability findings require matching ordered observations. Semicolon-terminated named entities from the HTML parser that decode to non-control characters, and non-control numeric entities map their decoded characters to the opening `&` (including entities that decode to two Unicode scalars); the remaining text retains its original position. Explicit `<wbr>` elements preserve adjacent text positions when the complete projection matches. Unsupported entities, hidden elements, unsupported or malformed markup, excessive nesting and other transformations leave locations unavailable. Finding coordinates remain relative to projected text; separate mapped positions point into the supplied source. These observations do not establish browser visibility or page completeness, and telemetry contains no content, paths or URLs.

### Plain-text source locations

For `text` and `paragraph-text` inputs, JSON reports include `source_locations`
for positional prose findings whose coordinates identify an actual character in
the unchanged input. Line and column are one-based Unicode-character coordinates.
Whole-document findings and invalid coordinates have no precise source location.
Opt-in source-mapping telemetry reports aggregate mapped and unmapped counts only.

### Markdown source locations

For Markdown CLI reports, finding `line` and `column` retain their existing
coordinates in extracted prose. The additive `source_locations` array supplies
original-file coordinates, indexed by `gate_id` and zero-based `finding_index`.
For single-line text nodes, semicolon-terminated entities and escaped ASCII
punctuation map when a bounded reconstruction matches the parser output exactly.
Decoded characters point to the entity's opening `&`; escaped punctuation points
to the punctuation after the backslash. Unsupported transformations and ambiguous
or multiline nodes can remain unmapped; unchanged edges can still be mapped.
These locations apply to positional prose gates and to `link_destinations`,
`heading_length` and `duplicate_headings` findings, not metadata, site or
collection findings. Soft-wrapped text maps directly only where the wrapped
source is unchanged. Multiline container prefixes, such as block-quote markers,
are mapped when every line in the text node matches exactly after those prefixes;
entities and escapes retain the conservative edge-only behavior. This mapping is
limited to 10,000 lines per node and otherwise falls back to its exact edges. Inline
code maps to its value inside the backticks. Literal image alt labels map to
their original Markdown text for both inline and reference images; transformed
alt labels remain unmapped. The map is local; opt-in telemetry includes only mapped and
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
produce findings that quote up to 200 characters and explain the issue. The
check rejects backslashes and invalid percent escapes (such as `%zz` or a trailing
`%`); encode literal percent signs as `%25`. Scheme-relative URLs beginning with `//` are rejected because
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
destinations without recording addresses in telemetry. Image destinations and raw HTML embedded in Markdown are outside this gate's scope; the explicit HTML adapter checks anchor destinations.

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

## Heading hierarchy

The opt-in `heading_hierarchy` gate flags a heading that skips a level from
its immediate predecessor, for example H2 followed by H4. It is advisory when
`blocking = false` and does not decide whether the overall outline is good.

```toml
[[gates]]
id = "editorial.heading-hierarchy"
kind = "heading_hierarchy"
blocking = false
# Optional: the level supplied by the surrounding page template (1 through 6).
initial_level = 1
```

Without `initial_level`, the first heading may have any level: this supports
fragments whose surrounding template is unknown. Set it only when the host
knows the immediately preceding heading level. Measurements include the configured
`initial_level` when supplied, and, for a complete observed sequence, the
`heading_count` and `skipped_levels`. A nonempty complete sequence also reports
`first_heading_level` and `deepest_heading_level`; those keys are omitted for
empty or unavailable sequences instead of reporting a misleading zero. Markdown
and static HTML adapters and
the EmDash Portable Text adapter provide levels, with source positions when
available. Markdown containing raw HTML and partially observed CMS headings
are `not_checked`: the projected sequence cannot establish all headings.
Static HTML results cover the supplied markup and scope; they do not establish
what a browser renders. Plain text and hosts without a complete ordered level
sequence are also `not_checked`. This gate is opt-in and does not change the
built-in presets.

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

The opt-in `sentence_character_length` gate uses the same sentence-boundary heuristic, delimiters and optional abbreviation exceptions as `sentence_length`, but limits Unicode scalar values instead of whitespace-separated words. It excludes sentence delimiters, following closing marks and leading/trailing whitespace while counting internal whitespace and punctuation. It does not count visual grapheme clusters and is not linguistic sentence segmentation. Measurements include `max_chars`, `sentence_count`, `longest_sentence_chars`, and `sentences_exceeding_limit`; missing text is `not_checked`.

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

The local `aexeo.preset_coverage` event reports the number of input documents,
those with evaluated preset reports, those with an error report, and those with
no report. Missing reports do not say why an assessment is absent: it may be
not configured, unsupported by the evaluator, or from an older stored result.
On evaluator error the per-document counts are zero and the event status is
`error`. Counters contain no document names, routes, gate IDs or content.

The `aexeo.preset_coverage` event also aggregates the number of reports with
mapping data and their eligible, mapped and unmapped finding counts. Older
bridges omit coverage; `mapping_reports` distinguishes that from zero findings.

For unchanged Portable Text blocks, EmDash additionally records the path to the
original text span in the evaluated CMS record and its UTF-16 offset. Paths are
snapshot-specific; re-evaluate after changes. Synthesized blocks and ambiguous,
cyclic or excessively large inputs do not receive a CMS source path. The local
`aexeo.cms_location_coverage` event reports eligible, mapped and unmapped counts
without paths, content or identifiers.


### Duplicate paragraphs

The opt-in `duplicate-paragraph-review` preset reports repeated paragraphs of
at least 40 Unicode scalar values after normalizing whitespace; comparison also
uses lowercase. The threshold is a configurable starting point, not a quality score.
Compose or override its `editorial.duplicate_paragraphs` gate for the content
being reviewed. It is advisory and requires an adapter that supplies paragraphs.
Neither the preset nor the underlying `duplicate_paragraphs` gate detects AI
authorship or semantic similarity; repeated passages can be intentional.

```sh
aexeo-cli content draft.md --input-format markdown --preset duplicate-paragraph-review --telemetry
```

```toml
[[gates]]
id = "editorial.duplicate_paragraphs"
kind = "duplicate_paragraphs"
max_occurrences = 1
min_chars = 40
case_insensitive = true
```

Both limits must be positive. `min_chars` counts Unicode scalar values after
whitespace normalization; `case_insensitive` uses Unicode lowercase conversion,
not full Unicode case folding. Findings identify each occurrence beyond the
configured maximum, without including the paragraph text. Measurements include
the total, evaluated and skipped paragraphs and occurrences exceeding the limit.
Missing paragraphs are `not_checked`. The check also reports `not_checked` if
a paragraph exceeds 16 KiB, there are more than 100,000 paragraphs, or its
10,000-key / 4 MiB distinct-key budget is exceeded; findings already observed
remain available. Locations are supplied where the adapter can map them.

### Experimental offline change receipts

`content-change --offline-receipt --evaluator-identity BUILD_ID` writes an
`aexeo.offline-decision-receipt/v1` object to `CONTENT_GATE_VERDICT_FILE` in
place of the usual v1 verdict. It is an Aexeo-specific experimental format,
not a `content-gate.verdict/v2`. A v1-only runner must reject it. The host must
supply an identity for its trusted evaluator build and independently establish
that it is correct; Aexeo does not authenticate the supplied value.

Use `--verify-offline-receipt PATH` with the same mode to compare the current
inputs and a fresh evaluation before writing a new receipt. Errors do not write
a new output; runners must check the exit code and not reuse an old file on
failure. This mode rejects `--verify-verdict`, `--extension-results` and
`--check-link-reachability`; external observations are not evaluated. It does
not prove collection completeness, input authenticity, build identity or
freshness at publication. Coordinate concurrent changes at the host boundary.
`--telemetry` reports an aggregate receipt-revalidation match boolean; with main scope it also reports the scope and eligible HTML entry count.

### Windowed sentence opening review

The optional `sentence-opening-window-review` preset provides an advisory window of six sentences with a maximum of two occurrences and no ignored words. The `sentence_opening_window` gate finds repeated literal openings in a bounded sliding window, including when other sentences intervene. It is advisory by default and independent of `repeated_sentence_openings`; compose both when useful.

```toml
[[gates]]
id = "editorial.opening_window"
kind = "sentence_opening_window"
opening_words = 1
window_size = 6
max_occurrences = 2
ignored_words = ["the", "le", "la"]
window_across_paragraphs = true
blocking = false
```

The window contains 2–64 sentences; `max_occurrences` must be positive and smaller than the window. It resets at paragraph boundaries unless `window_across_paragraphs` is true. Consecutive leading words listed in `ignored_words` are removed before selecting the opening; comparison ignores case, surrounding punctuation and Unicode normalization differences while preserving accents. The gate checks shorter windows at the start of a paragraph or document as well. Short, ignored-only and quoted sentences still occupy a window slot; unclosed quotations produce `not_checked`. Delimiters and abbreviation exceptions can be configured as for sentence-length checks. Unavailable paragraphs, unsupported openings, no window with enough evaluable openings to exceed the configured limit and resource limits remain `not_checked`; findings already observed are retained. Findings are suggestions for editorial review, not evidence of authorship.

### Repeated sentence openings

The optional `sentence-openings-review` preset flags the third and subsequent consecutive sentences in a paragraph that begin with the same two words. It is advisory and can be composed or overridden in a TOML preset:

```toml
extends = ["sentence-openings-review"]

[[gates]]
id = "editorial.sentence_openings"
kind = "repeated_sentence_openings"
opening_words = 2
max_consecutive = 2
blocking = false
# delimiters = ".!?"
# abbreviations = ["Dr.", "e.g."]
```

Comparison ignores case and punctuation around whitespace-separated opening words and compares canonically equivalent Unicode spellings together while preserving accents. Sentence delimiters and abbreviations follow the sentence-length gates; sentence segmentation is heuristic, and no language is selected automatically. Paragraph boundaries reset the sequence. Sentences with fewer than the configured number of opening words reset the sequence and are counted as `sentences_too_short` when their characters belong to a conservative Latin subset, including common accented letters and typographic quotation marks. Other short sentences can be `not_checked` because their word boundaries may be uncertain. Unrecognizable openings, unavailable paragraphs and resource limits also produce `not_checked` rather than a complete pass; findings already observed remain in the report. Measurements describe counts, and opt-in telemetry contains aggregate timings without content. This is a prompt for editorial judgment, not an assessment of authorship or AI detection.

## Structured editorial pacing

The opt-in `news-pacing` and `opinion-pacing` presets contain an advisory,
non-blocking `editorial_pacing` gate. They can be inherited or overridden by
other portable content presets. Their `news` and `opinion` composition profiles
are starting points, not quality standards or measures of authorship. The
`news` hook has at most 45 Unicode scalar values and its context has 160–240;
`opinion` permits a 90-character hook and an argument of at least 120. These
bounds are independent of words and sentences and can be inappropriate for a
particular story or language.

Export the JSON Schema for an agent, validate its structured output, and
assemble the exact content without model-generated transitions:

```sh
aexeo-cli composition --profile news --schema > schema.json
aexeo-cli composition --profile news --input composition.json
aexeo-cli composition --profile news --input composition.json --assemble > draft.txt
aexeo-cli content draft.txt --preset news-pacing --evidence-source composition=composition.json --format json
```

The schema requires every field and rejects unknown fields. JSON Schema length
bounds count Unicode scalar values, not visual characters or words. The CLI
rejects duplicate JSON keys and limits input sizes. `--assemble` only emits text
for an input satisfying the profile and does not add a trailing newline. If an
editor intentionally accepts a bounds violation, `--allow-advisory-violations`
permits assembly of well-formed fields while preserving the later gate finding;
missing, unexpected or incorrectly typed fields are still rejected. The
composition may include private content; telemetry is opt-in and reports only
aggregate counts and status.

To check an existing document, pass `--document draft.txt` to `composition`.
The document gate checks the exact adapter-supplied source (or, when unavailable,
the host-declared complete text projection) against the assembled snapshot before
making findings. A matching projection does not establish rendered-page equivalence
or preserve formatting. The optional gate parameter `projection_separator` specifies
how a complete adapter text projection joins fields when the original source is
unavailable. Built-in and exported pacing presets use `"\\n"` for EmDash; hosts
with another projection can override it. Original source bytes always use the
profile's assembly separator. Missing, malformed, oversized or
nonmatching snapshots and unavailable source text are `not_checked`; they are
not evidence that a document passed. Recreate the structured snapshot after
editing. This exact-byte check does not authenticate content or verify facts.

Custom profiles use TOML with ordered `[[fields]]`. Each string can specify
`min_characters` or `max_characters`; a list is declared with a nonempty
`allowed_counts` array, such as `[{ max = 2 }, { min = 4 }]`, and contains
strings. List items are assembled in order using the same profile separator.
A project can pass a profile path to `composition --profile` and embed the
same profile as a TOML table in a custom `editorial_pacing` gate. The gate's
`profile` can instead be the built-in name `news` or `opinion`. Gate severity
and blocking are configurable; the defaults are warning and non-blocking.
These structural checks do not assert that a field contains one sentence, that
its role is semantically accurate, or that an AI detector will give any result.

For example, a custom preset can inherit other checks and replace the pacing
policy by gate ID:

```toml
version = 1
name = "My news policy"
extends = ["article-quality", "news-pacing"]

[[gates]]
id = "editorial.pacing"
kind = "editorial_pacing"
source = "composition"
profile = { version = 1, name = "my-news", fields = [
  { name = "opening_hook", max_characters = 70 },
  { name = "context", min_characters = 80, max_characters = 280 },
  { name = "examples", allowed_counts = [{ max = 2 }, { min = 4 }] },
] }
```

For a custom profile in `profile.toml`, export the matching gate preset with
`aexeo-cli composition --profile profile.toml --emit-preset > pacing.toml`.
Load it as a named `--preset-source` and compose it with other presets. This
uses the exact profile used for schema generation instead of manually copying
its bounds. A gate override replaces the whole gate, rather than merging individual fields.
Profiles and structured input are bounded; the JSON Schema cannot express all
of Aexeo's aggregate resource limits. A schema-valid response can therefore
still be refused if it exceeds those limits.

### Structured list item counts

The optional `list_item_count` gate asks for editorial review of lists with configured direct item counts. It does not infer authorship or quality and never deletes list items. Nested lists are counted separately. Configure it in a portable preset:

```toml
[[gates]]
id = "editorial.review_three_item_lists"
kind = "list_item_count"
flagged_counts = [3]
blocking = false
```

The Markdown CLI and content-change evaluator supply observations when the list structure is fully visible. The single-content CLI also observes supported static HTML lists in the selected document or main scope; it checks the source structure against the parsed HTML projection and reports `not_checked` when it cannot establish complete list structure. Embedded HTML in Markdown, plain text, unsupported adapters and incomplete snapshots remain `not_checked`; a host adapter JSON can supply explicit complete list observations; an observed document without lists passes. Markdown and supported HTML CLI source locations, and Markdown content-change finding locations, identify the original list openings. Static HTML does not establish browser-rendered visibility or page completeness. Hosts may supply complete observations through the additive `evaluate_with_structured_lists` API. For Rust EmDash hosts, `evaluate_content_with_list_completeness` accepts an explicit `list_structure_complete` assertion; set it only when all list blocks, including custom components, are represented in order. Existing CMS calls make no such assertion and therefore report this gate as `not_checked`. Observations are advisory. The projected finding coordinates still use a placeholder; inspect the separate CLI source locations or change-verdict locations for precise list openings. EmDash reports can identify the first nonempty span of the list-opening block; when that block has no text, the finding remains unmapped. These coordinates refer only to the evaluated snapshot.

When complete list structure is observed, the gate reports `lists_observed`,
`lists_matching_counts` and `largest_list_items`. Matching counts include all
observations even when the finding limit truncates the displayed findings; the
largest count is zero for an observed document without lists. These observed
measurements are omitted when list structure is unavailable or invalid.

With opt-in content telemetry, CLI and EmDash emit `list_review` counts for
configured, measured and not-checked list gates. Observed and matching list
counts are included only when at least one gate measured them; they sum
observations per gate, so the same list is counted once for each gate. These
aggregates contain no content, paths, gate identifiers or configured terms.

To opt in through the configured EmDash plugin, set `contentPreset.listStructureCompleteByCollection = ["blog_posts"]` only after verifying that the host projects every list block in those collections in order, including any custom components. A missing body or incomplete existing body capabilities still reports `not_checked`. This assertion defaults to off. Direct WASM hosts can set `list_structure_complete: true` on the document under the same condition.
### Repeated transition openings

The opt-in `transition-review-en`, `transition-review-fr`, `transition-review-de`,
`transition-review-it` and `transition-review-ar` presets flag configured
connective expressions at the start of a sentence when their count
in a paragraph exceeds `max_per_paragraph`. Choose the preset for the actual
content language; locale is not inferred. The `sentence_opening_terms` gate
accepts a custom `terms` list, `max_per_paragraph`, `delimiters` and sentence
`abbreviations`; it is advisory and non-blocking by default. It requires
paragraphs and reports `not_checked` when the adapter cannot provide them.
Sentence segmentation is heuristic. These style checks conservatively skip
sentences containing or inside common double, curly or French quotation marks.
A quotation left open at a paragraph boundary makes the result `not_checked`;
straight single quotes are not tracked because they also mark contractions.
Quotation recognition is heuristic and cannot establish every quoted span.
The German, Italian and Arabic phrase lists are small, provisional examples,
not comprehensive language models or validated measures of translation quality.
The Arabic starter also recognizes the Arabic question mark as a sentence
boundary; it does not provide full Arabic sentence segmentation or normalize
diacritics. Choose or override expressions and delimiters for the publication.
The gate never removes or rewrites a transition: it may express an essential causal or
logical relationship. The finding is an invitation to review the paragraph,
not evidence of AI authorship or a detector outcome.

```toml
version = 1
name = "News transitions"
extends = ["transition-review-en"]

[[gates]]
id = "editorial.transition_openings"
kind = "sentence_opening_terms"
terms = ["Moreover", "Additionally", "In conclusion"]
max_per_paragraph = 2
blocking = false
```

### Punctuation review

The optional `punctuation-review` preset counts em dashes in each observed
paragraph and asks for review above the configured limit. The
`repeated_punctuation` gate accepts a single Unicode punctuation or symbol
`mark` and a `max_per_paragraph` limit. It is advisory and requires observed
paragraphs; missing paragraphs are `not_checked`. Counts include punctuation
inside quotations and other inline prose, and are not linguistic analysis. No
punctuation is replaced. The composable `news-style-en`, `news-style-fr`, `opinion-style-en`, and
`opinion-style-fr` starters, as well as the German and Italian `news-style-de`,
`news-style-it`, `opinion-style-de` and `opinion-style-it`, `news-style-ar` and `opinion-style-ar` starters, combine this
gate with transition and opening review; their different
limits are examples for projects to override, not universal standards.
They also compose sentence variance and opening-window reviews. News uses a six-sentence
window with a six-word standard-deviation threshold and at most two identical openings;
opinion uses eight sentences, a five-word threshold and at most three identical openings.
These advisory examples are not calibrated recommendations. Override either gate in your
preset to match your audience, language and editorial judgment.

Arabic rhythm starters opt into `allow_arabic_word_boundaries = true` for
`similar_sentence_lengths`, `sentence_length_variance`, and
`sentence_opening_window`; it is also available on
`prose_sentence_length_variance`. The default stays false. The option accepts a
limited set of Arabic-script characters and punctuation and counts
whitespace-separated tokens; it is not language detection, morphological word
segmentation or a validated quality score. Configure `delimiters` separately,
including `؟` when appropriate. Unsupported characters in a sentence still
make affected checks `not_checked`. Other scripts and unusual Arabic marks may
remain unassessed. Rust callers constructing these gate variants must supply
this new field; serialized presets omitting it keep the previous behavior.

### English trailing clause review

`trailing-ing-review-en` flags a sentence whose last comma is followed by an
ASCII word ending in `ing`. This is a deliberately narrow surface heuristic:
it does not parse grammar, identify causation, or establish that the clause is
unnecessary. It can miss clauses and flag valid wording. Use it only for English
content, review any candidates in context, and retain all relevant information.
The gate does not delete clauses. Unavailable paragraphs remain `not_checked`.

### Sentence rhythm review

`sentence-rhythm-review` flags runs of four sentences whose whitespace-separated
word counts differ by no more than two words. Configure `run_length` (2–64),
`tolerance_words`, sentence delimiters and abbreviations to fit the content.
The check resets at paragraph boundaries. It is advisory and uses heuristic
sentence boundaries; unsupported scripts, unavailable paragraphs and bounded
input limits are reported as `not_checked`. Similar lengths may be intentional
and are not evidence of AI authorship.

### Sentence length variance review

The opt-in `sentence-variance-review` preset reports rolling windows of six
eligible sentences with a population standard deviation below six words. This
initial threshold is advisory, not a calibrated quality standard. Sentences
can span paragraph boundaries; quotations and unsupported word boundaries break
a window. Insufficient observations, incomplete quotes, unsupported scripts,
unavailable paragraphs and input limits are `not_checked`. Segmentation and
word counts are heuristics; repeated lengths may be an intentional choice.

Set `reset_at_paragraph = true` to keep each rolling window within one
observed paragraph. The default is `false` for compatibility. In paragraph
mode, `paragraphs_without_windows` counts paragraphs that produced no complete
window. If any paragraph cannot form a window, the gate reports `not_checked`
while retaining findings from other paragraphs. This option does not identify
lists or distinguish their items from prose; that depends on the adapter's
paragraph projection. Rust callers constructing `ContentGate::SentenceLengthVariance`
directly must set `reset_at_paragraph` explicitly.

Compose or override the fixed setting:

```toml
version = 1
name = "My variance review"
[[gates]]
id = "editorial.sentence_variance"
kind = "sentence_length_variance"
window_size = 6
min_stddev_words = 6
blocking = false
```

Alternatively replace `min_stddev_words` with all three seeded range settings:

```toml
random_min_stddev_words = 6
random_max_stddev_words = 10
random_seed = 42
```

Bounds are inclusive integer word counts from 1 to 1000; the window size is
2 to 64 sentences. The seed is explicit. A versioned SHA-256 derivation of the
seed and gate ID chooses one threshold for the gate, not for each sentence or
revision. The same policy therefore yields the same threshold across adapters;
change the seed deliberately to select another. The report records
`effective_min_stddev_words`, `windows_checked` and `low_variance_windows`.
Neither the configuration nor findings establish authorship or guarantee any
outcome from AI detectors. Review the passage before proposing an edit.

### Prose-only sentence variance

The opt-in `prose_sentence_length_variance` gate accepts the same settings as
`sentence_length_variance`, but excludes list paragraphs and breaks rolling
windows at each excluded paragraph. Its `list_paragraphs_excluded` measurement
reports the number excluded. The host must call
`ContentPreset::evaluate_with_paragraph_list_items` with one classification per
projected paragraph, in the same order, and attest that the projection and
classification are complete. Missing or mismatched classification is
`not_checked`; it is never assumed to mean there are no lists. This API does
not establish source completeness on the host's behalf.

EmDash/WASM supplies this observation only when paragraph coverage is available
and the host explicitly attests `list_structure_complete`; the host must account
for omitted custom components before making that assertion. Invalid list nesting
or list items omitted from paragraph projection remain `not_checked`. Configured
plugins can use `listStructureCompleteByCollection` only for collections whose
complete projection the host can establish.

The `prose-variance-review` starter is advisory, with a six-sentence window
and a six-word threshold. Compose it after an existing style profile to replace
that profile's variance gate while retaining its other checks, for example:

```toml
version = 1
name = "News with prose-only variance"
extends = ["news-style-en", "prose-variance-review"]
```

Override the gate in a project preset to select different fixed or seeded
thresholds. These defaults are prompts for editorial review, not a measure of
authorship or factual accuracy.

The CLI and content-change entry points supply this observation for Markdown
and HTML. Unsupported or incomplete structures, and plain-text inputs that do
not establish list structure, remain `not_checked`. Opt-in CLI telemetry reports
aggregate measured gates, excluded list paragraphs and checked windows without
content or gate identifiers. The adapted in-process incremental evaluator can
reuse unchanged Markdown and HTML reports when every evaluated item has complete
list observations. Plain text, unrepresented list items and other missing
capabilities make the incremental result `not_checked`. Existing sentence-variance
configurations retain their behavior.

### Possible inline triples

The opt-in `inline-triples-en` and `inline-triples-fr` presets report surface
patterns resembling a three-item inline enumeration with a configurable
conjunction. This is not a dependency parser or a complete list inventory: it
can miss or misclassify examples, does not inspect structured list boundaries,
and cannot establish whether an enumeration is repetitive or inappropriate.
It never deletes or splits an item. Select the correct language explicitly and
review candidates in context; unavailable paragraphs are `not_checked`.

### Advisory comparison of content-gate findings

`content-change --baseline-verdict OLD.json` adds an
`aexeo.advisory-entity-baseline.v1` extension to the new verdict. Findings are
matched by entity ID, rule ID and subject, one occurrence at a time. Explicit
`previous_entity_id` rename entries map old identities to new; ambiguous mappings
are refused. Findings without an entity ID cannot match. Existing site-audit
baselines retain their historical path-based behavior.

This comparison is informational only: it never changes the verdict or exit code.
It does not authenticate the old verdict or verify policy equivalence, source
freshness, scope, or completeness. Unmatched old findings appear in
`unconfirmed_resolutions`; they may reflect missing or unchecked evaluations,
so `comparison.resolved_findings` is empty. Do not use this extension to
suppress findings or authorize publication. The extension records
both statuses, omitted-finding counts and any available policy hashes so a host
can inspect the underlying limitations. `--baseline-verdict` cannot be combined
with experimental offline receipts or observation export.

### Advisory identity comparison for site audit baselines

Site audits do not currently carry a host's stable content entity IDs. To compare
existing site audit or baseline artifacts across a path change, provide one
host-produced manifest for each artifact:

```json
{
  "schema": "aexeo.site-entity-map/v1",
  "namespace": "cms:example",
  "scope": "published-en",
  "entries": [{ "path": "articles/example.html", "entity_id": "post:123" }]
}
```

Run `aexeo-cli diff before.json after.json --baseline-entities before-map.json
--current-entities after-map.json --format json`. Paths must exactly match the
finding paths in the corresponding artifact. Namespace and scope are explicit
host claims and must match between manifests. Each path and entity ID must be
unique within its manifest. Mapped findings use the stable ID, rule ID and
subject for comparison; unmapped findings use their original paths. A mapped
finding never matches an unmapped one. Existing baseline files remain readable.

The additional `entity_comparison` is advisory. The normal path-based result and
exit status remain unchanged, as do `check --regressions-only` and runtime
verification. Aexeo does not verify the host's manifest provenance, scope,
completeness, policy or binding to either artifact; a reported resolution can
therefore reflect an incomplete audit. The host must establish these facts
independently before using any comparison in a publication decision. In
particular, these manifests do not attest freshness or ownership of an identity.

### Experimental incremental batches

`aexeo-cli content-incremental --input batch.json` (or `--input -` for a
JSON batch on stdin) evaluates independent proposals
against the **same** host-provided base and resolved preset in one process. It
prepares the base once and reuses unchanged item reports. Each proposal is an
alternative against that base, not a sequence of edits. The bounded JSON input
uses schema `aexeo.content-incremental-batch/v1-experimental`, with `snapshot`
(a candidate snapshot), `preset` (a named built-in preset), optional
`preset_sources` (name-to-TOML map, overriding built-ins), `mode` (`plain`,
`paragraph_text`, `adapted`, or `adapted_main`), and `proposals` (an array of
objects with `change` and `after`, as in `content-impact`). The batch is limited
to 16 MiB, 256 base items or changed projections per proposal, and 16 proposals.
Only inline change content is supported. Site, source and external gates remain
unsupported and are `not_checked`. A complete synthetic example is
[`incremental-batch.json`](../crates/aexeo-cli/tests/fixtures/incremental-batch.json);
it exercises a composed item and collection preset and is tested through the
CLI. The example is illustrative; it does not establish a real site's inventory.
Pipe it without a temporary file using:

```sh
cat crates/aexeo-cli/tests/fixtures/incremental-batch.json | aexeo-cli content-incremental --input -
```

Both file and stdin input use the same byte limit and strict duplicate-key
validation. Results stream to stdout; telemetry, when enabled, goes to stderr.

An optional `base_metadata` object maps every base entity ID to a complete map
of string fields, or to `null` if its metadata was not observed. An empty map
means metadata was observed but no fields were present. The outer object must
be omitted or be a map; `null` is invalid. For each created, updated or renamed
entity the change entry must provide its own complete `metadata` map. Missing
projections stay unavailable. The host must bind metadata to the corresponding
revision and establish its authenticity and freshness. Item metadata gates and
mixed item/collection presets require `adapted` or `adapted_main` mode.
Collection-only presets may use any mode. The supported collection gate is
`unique_collection_metadata`; other collection gates are not inferred.

Stdout is newline-delimited JSON, one result per proposal, in input order. Each
record has `schema`, `proposal_index`, and `result`. An `evaluated` result has
item reports and `evaluated`/`reused` counts; individual gates can still report
findings. A `collection` result contains a `collection` report, and a `composed`
result contains separate `items` and `collection` results. An unavailable
collection gate can retain observed findings while still making the proposal
not checked. Exit 0 means every proposal was evaluated without blocking failures; advisory
findings can still exist. Exit 1 means at least one blocking gate failed. Exit 2
means at least one proposal was not checked; invalid input or execution errors
also produce a nonzero exit. Consumers must inspect all records and the exit
status, and must not treat a partial stream as complete. `--telemetry` writes
aggregate counts and timing to stderr without source text. Both the input and
result reports may contain private information.

The host asserts snapshot completeness and revision freshness; Aexeo does not
authenticate them. These experimental item and collection reports are not a
publication verdict and do not establish site, source, or publication safety.
