# Portable content gates

The content evaluator runs locally without a network or model. It is separate from the existing site audit and does not assess source support or factual correctness. A report records each gate as `passed`, `failed`, or `not_checked`; the last means an adapter could not supply a required capability.

```sh
aexeo-cli content article.md --preset article --format json
printf 'Draft text' | aexeo-cli content - --preset glossary
aexeo-cli content --preset glossary --explain
```

The CLI reads the supplied UTF-8 bytes as text. It does not parse Markdown or exclude front matter, markup, links or code blocks from word counts or term searches. Word counts split on whitespace, and forbidden terms are exact, case-sensitive substrings. Locations refer to the supplied text. EmDash projects blocks and spans to plain text; its locations refer to that projection, not the original structured document.

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
id = "metadata.required"
kind = "required_metadata"
fields = ["title", "description"]
```

Save this as `product.toml`, then run:

```sh
aexeo-cli content article.md --preset product --preset-source product=product.toml --metadata metadata.json --format json
```

Metadata must be a JSON object with string values. Without `--metadata`, metadata gates are `not_checked`, rather than treating missing input as an empty set of fields. An empty object indicates that metadata was supplied but contains no fields.

`extends` is a list of named presets. Parents are applied left to right; gates with an existing ID replace the earlier definition in place, and new gates append. `disable = ["gate.id"]` removes inherited gates and rejects unknown IDs. Supply each custom parent with a separate `--preset-source name=path`; built-ins are already available. Cycles, unknown parents, duplicate local IDs, unknown fields, invalid bounds and unsupported versions fail resolution. A preset's `name` is its report label, while its source name is the key used in `extends` and `--preset`.

Each gate may set `severity` to `error`, `warning` (default) or `note`, and `blocking` to true (default) or false. Status always reports the gate result; the CLI exits 1 for a failed blocking gate, 2 if a blocking gate was not checked (even if another blocking gate failed), and 0 otherwise. Non-blocking gates still appear in the report, including when they could not be checked. Invalid configuration or I/O errors use the CLI's internal-error path. No content is automatically changed.

The EmDash Rust bridge and `evaluateContent` WASM export accept the same named source map and preset, while retaining the existing site-audit APIs. Enabling those gates in the EmDash plugin's existing save workflow requires a separate integration.
