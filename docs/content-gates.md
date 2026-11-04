# Portable content gates

The content evaluator runs locally without a network or model. It can include the existing site audit through a site gate. It does not assess source support or factual correctness. A report records each gate as `passed`, `failed`, or `not_checked`; the last means an adapter could not supply a required capability.

```sh
aexeo-cli content article.md --input-format markdown --preset article --format json
printf 'Draft text' | aexeo-cli content - --preset glossary
aexeo-cli content --preset glossary --explain
aexeo-cli content --list-presets
aexeo-cli content --preset seo-website --site ./dist --no-text --format json
```

By default the CLI reads the supplied UTF-8 bytes as raw text, including Markdown syntax. With `--input-format markdown`, it parses CommonMark with GFM extensions and projects text and headings while excluding front matter, code and HTML; link labels and image alt text are included. Word counts split on whitespace, and forbidden terms are exact, case-sensitive substrings. Locations in raw-text mode refer to the supplied text. Markdown and EmDash findings refer to their extracted plain-text projection, not source line positions.

The built-in `seo-website` preset wraps the existing static SEO/GEO audit as one site-scoped gate and uses its existing configuration and policy. Pass `--site-config` to select a site config. It preserves the audit findings and their severities. The gate fails if the audit produces any findings, including warnings. A text-only adapter reports it as `not_checked`. For custom presets, `site_group` composes individual native website rule groups, for example:

```toml
[[gates]]
id = "site.html"
kind = "site_group"
group = "html"
```

Use `aexeo-cli content --list-site-groups` for supported group names. The existing site configuration's group switches, suppressions, and severity overrides apply. A group disabled by that configuration is `not_checked`. The `seo-website` preset remains a single site audit gate, preserving its existing complete findings, including deployment diagnostics.

The built-in `article` preset requires at least one whitespace-separated word. `glossary` inherits it and sets a maximum of 500 words. These deliberately minimal examples are starting points, not comprehensive editorial standards.

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

Metadata must be a JSON object with string values. Without `--metadata`, metadata gates are `not_checked`, rather than treating missing input as an empty set of fields. An empty object indicates that metadata was supplied but contains no fields.

`extends` is a list of named presets. Parents are applied left to right; gates with an existing ID replace the earlier definition in place, and new gates append. `disable = ["gate.id"]` removes inherited gates and rejects unknown IDs. Supply each custom parent with a separate `--preset-source name=path`; built-ins are already available. Cycles, unknown parents, duplicate local IDs, unknown fields, invalid bounds and unsupported versions fail resolution. A preset's `name` is its report label, while its source name is the key used in `extends` and `--preset`.

Each gate may set `severity` to `error`, `warning` (default) or `note`, and `blocking` to true (default) or false. Status always reports the gate result; the CLI exits 1 for a failed blocking gate, 2 if a blocking gate was not checked (even if another blocking gate failed), and 0 otherwise. Non-blocking gates still appear in the report, including when they could not be checked. Invalid configuration or I/O errors use the CLI's internal-error path. No content is automatically changed.

The EmDash package exposes `evaluateContent` from `@aeptus/aexeo-emdash/content`; the Rust bridge and WASM API accept the same named source map and preset. The document adapter has no site capability, so site gates are `not_checked`. Configured-mode plugins can opt into a document preset for both save hooks and refreshes:

```ts
aexeoPlugin({
  contentPreset: { name: "article", byCollection: { glossary: "glossary" } },
})
```

For custom presets, add `sources: { custom: "...TOML..." }` and select `name: "custom"`. `name` is the fallback preset; `byCollection` selects a named preset for a specific collection on both save and refresh. The existing SEO findings remain enabled. A gate the document adapter cannot check causes an explicit incomplete-evaluation error rather than being silently omitted. The sandboxed plugin's sidecar workflow does not yet support this option.

## Local latency measurement

Run `cargo run --release --locked --offline -p aexeo-core --example content_bench` to measure the content gates alone and Markdown parsing plus evaluation over a synthetic 29 KB document. It reports p50 and p95 across 100 iterations; figures depend on hardware and concurrent load, exclude CLI process startup, and are not a CI threshold or a guarantee for other documents.
