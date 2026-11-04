# Content architecture and compatibility

Aexeo keeps the existing website audit as the source of SEO/GEO rules. Portable presets compose content gates or invoke the existing site audit and its individual rule groups. The content engine operates on supplied capabilities and does not fetch sources, invoke a model or infer that an unavailable check passed.

## Inputs and adapters

- Raw text and stdin provide text and original source. The Markdown adapter projects prose and headings; its locations refer to the projection.
- EmDash portable text provides text, headings and metadata. Configured plugins evaluate locally through WASM. Sandboxed plugins use the authenticated sidecar, which holds preset configuration.
- A static site and its existing `Config` supply the site capability. `seo-website` delegates to the full existing audit; `site_group` selects individual registered groups and applies the existing policy.
- `RuntimeAudit::evaluate_preset` evaluates a preset on the site produced by a crawl while retaining the crawl status, truncation reason and both crawl and original audit findings. A partial crawl does not establish a complete review. The existing crawl CLI and artifact format are unchanged. The `content --crawl-url` CLI exposes this adapter with an explicit crawl-status envelope and rejects an incomplete crawl as unsupported.

Each gate reports its scope, required capability, blocking policy, findings and one of `passed`, `failed` or `not_checked`. Presets resolve deterministically from named TOML sources, support parent composition and explicit overrides, and reject invalid or unknown configuration. Built-in TOML files are embedded so native and WASM callers use the same rules.

## Compatibility boundaries

Existing `check`, `crawl`, `verify` and EmDash behavior remains the default. Content presets are opt-in. The full website preset delegates to the existing audit to preserve diagnostics, ordering and policy; a parity test compares its complete findings against the legacy site evaluator. Runtime tests cover the crawl commands and artifacts; bridge tests compare Markdown and EmDash projections and preserve existing rule switches, severity overrides and suppressions.

The standalone content report is a new contract. Advisory gate failures and missing non-blocking inputs are visible even when the CLI exits successfully. Source support, factual correctness, fairness and writing quality as a whole are not established by a passing deterministic check.

## Scope decisions

TOML is the first portable format: adding YAML would duplicate parsing and validation paths without adding an evaluation capability, and can be added behind the same source boundary later. Custom presets compose the built-in gate types; arbitrary executable extensions require a separately designed isolation and compatibility contract. Semantic evaluation and fact/source verification remain later work, after deterministic gates, evidence and review behavior have been established.
