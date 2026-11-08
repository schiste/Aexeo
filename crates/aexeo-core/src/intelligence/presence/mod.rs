// Layer-4 entity-presence diagnostic. Five free public APIs query
// the configured organization in the truth manifest and report
// what the open web actually shows. Aexeo deliberately surfaces
// this layer; it does not score it.
//
// Used on two surfaces:
//   - aexeo-cli `intelligence presence`: human/JSON output for
//     local + CI use
//   - @aeptus/aexeo-emdash: the plugin currently has its own TS
//     impl; planned migration to call into this module via the
//     bridge keeps the wire format identical (camelCase JSON).
//
// The five sources:
//   - Wikipedia        — OpenSearch, one language edition per run
//   - Wikidata         — wbsearchentities
//   - GitHub           — /users/<handle>
//   - RDAP             — rdap.org redirector
//   - Common Crawl     — CDX index of the newest listed crawl
//
// Network code lives behind the `net` feature; the WASM bridge
// builds without it. The pure helpers (formatting, host parsing,
// fuzzy match) and the public types are always available.

mod types;
mod util;

#[cfg(feature = "net")]
mod sources;

pub use types::{
    DEFAULT_WIKIPEDIA_LANGUAGE, EntityInput, SOURCE_ORDER, SourceResult, SourceStatus,
    entity_from_manifest, is_wikipedia_language, source_label,
};
pub(crate) use util::iso_from_unix_seconds;
pub use util::{
    days_since, extract_host, format_age, format_cdx_timestamp, fuzzy_match, iso_now,
    sanitize_github_handle,
};

#[cfg(feature = "net")]
pub use sources::check_all_sources;
