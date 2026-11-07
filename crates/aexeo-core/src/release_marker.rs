//! Read the release marker each crawled page serves, and attest what a crawl
//! saw. The marker is an observation of the site, never a caller's claim.

use std::collections::{BTreeMap, BTreeSet};

use aexeo_contracts::{
    AuditArtifact, RELEASE_DISTINCT_VALUES_CAP, ReleaseAttestation, ReleaseMarkerSource,
};
use serde::{Deserialize, Serialize};

use crate::site::markup::{attr_value, mask_non_markup, next_open_tag, open_tag_end};

/// Longest marker value kept, in characters; the rest is cut off.
pub const MAX_RELEASE_MARKER_CHARS: usize = 128;

/// Response headers that carry credentials or session state. Their values are
/// never recorded, whatever the caller asks.
const CREDENTIAL_HEADERS: [&str; 6] = [
    "authorization",
    "cookie",
    "proxy-authenticate",
    "proxy-authorization",
    "set-cookie",
    "www-authenticate",
];

/// Where to read a page's release marker. When both are set, a usable header
/// value wins and the meta is read only for pages without one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseMarkerSpec {
    header: Option<String>,
    meta: Option<String>,
}

impl ReleaseMarkerSpec {
    /// `None` when neither is requested. Header names are matched without
    /// regard to case; a header that carries credentials is refused.
    pub fn new(header: Option<&str>, meta: Option<&str>) -> Result<Option<Self>, String> {
        let header = header.map(|name| name.trim().to_ascii_lowercase());
        if let Some(name) = &header {
            if name.is_empty() || !name.bytes().all(is_header_name_byte) {
                return Err(format!(
                    "--release-header {name:?} is not an HTTP header name"
                ));
            }
            if CREDENTIAL_HEADERS.contains(&name.as_str()) {
                return Err(format!(
                    "--release-header {name} carries credentials and is never recorded"
                ));
            }
        }
        let meta = meta.map(|name| name.trim().to_string());
        if let Some(name) = &meta
            && (name.is_empty() || name.chars().any(|ch| ch.is_whitespace() || ch.is_control()))
        {
            return Err(format!("--release-meta {name:?} is not a meta name"));
        }
        Ok((header.is_some() || meta.is_some()).then_some(Self { header, meta }))
    }

    /// The page's marker: the header value if usable, else the first
    /// `<meta name>` outside scripts, styles and comments.
    pub fn observe(&self, headers: &BTreeMap<String, String>, html: &str) -> Option<String> {
        self.header
            .as_deref()
            .and_then(|name| {
                headers
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .and_then(|(_, value)| sanitize(value))
            })
            .or_else(|| {
                self.meta
                    .as_deref()
                    .and_then(|name| meta_content(html, name))
                    .and_then(|value| sanitize(&value))
            })
    }
}

fn is_header_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// Trimmed, free of control characters and at most `MAX_RELEASE_MARKER_CHARS`.
fn sanitize(value: &str) -> Option<String> {
    let cleaned = value
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>();
    let bounded = cleaned
        .trim()
        .chars()
        .take(MAX_RELEASE_MARKER_CHARS)
        .collect::<String>();
    let bounded = bounded.trim_end();
    (!bounded.is_empty()).then(|| bounded.to_string())
}

/// `content` of the first `<meta>` whose `name` matches, ignoring case.
fn meta_content(html: &str, name: &str) -> Option<String> {
    let markup = mask_non_markup(html);
    let mut from = 0;
    while let Some(start) = next_open_tag(&markup, "meta", from) {
        let end = open_tag_end(&markup, start)?;
        let tag = &markup[start..=end];
        if attr_value(tag, "name").is_some_and(|value| value.eq_ignore_ascii_case(name)) {
            return attr_value(tag, "content");
        }
        from = end + 1;
    }
    None
}

/// Count the markers of the artifact's pages, which are its successfully
/// fetched HTML pages. A value is attested only when every one of them
/// served the same non-empty marker.
pub fn attest_release(spec: &ReleaseMarkerSpec, artifact: &AuditArtifact) -> ReleaseAttestation {
    let mut pages_with_marker = 0;
    let mut pages_without_marker = 0;
    let mut values = BTreeSet::new();
    for page in artifact.site.iter().flat_map(|site| &site.pages) {
        match page
            .crawl_response
            .as_ref()
            .and_then(|response| response.release_marker.as_deref())
        {
            Some(value) => {
                pages_with_marker += 1;
                values.insert(value.to_string());
            }
            None => pages_without_marker += 1,
        }
    }
    let value = (pages_with_marker > 0 && pages_without_marker == 0 && values.len() == 1)
        .then(|| values.first().cloned())
        .flatten();
    let count = values.len();
    let (source, name, meta_fallback) = match (&spec.header, &spec.meta) {
        (Some(header), meta) => (ReleaseMarkerSource::Header, header.clone(), meta.clone()),
        (None, meta) => (
            ReleaseMarkerSource::Meta,
            meta.clone().unwrap_or_default(),
            None,
        ),
    };
    ReleaseAttestation {
        source,
        name,
        meta_fallback,
        value,
        pages_with_marker,
        pages_without_marker,
        distinct_values: values
            .into_iter()
            .take(RELEASE_DISTINCT_VALUES_CAP)
            .collect(),
        distinct_value_count: (count > RELEASE_DISTINCT_VALUES_CAP).then_some(count),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn neither_flag_means_no_spec_and_credential_headers_are_refused() {
        assert_eq!(ReleaseMarkerSpec::new(None, None), Ok(None));
        assert!(ReleaseMarkerSpec::new(Some("Set-Cookie"), None).is_err());
        assert!(ReleaseMarkerSpec::new(Some("Authorization"), None).is_err());
        assert!(ReleaseMarkerSpec::new(Some("x release"), None).is_err());
        assert!(ReleaseMarkerSpec::new(None, Some(" ")).is_err());
        assert!(ReleaseMarkerSpec::new(Some("X-Release"), Some("release")).is_ok());
    }

    #[test]
    fn a_header_marker_is_matched_without_case_and_sanitized() {
        let spec = ReleaseMarkerSpec::new(Some("X-Release"), None)
            .unwrap()
            .unwrap();
        assert_eq!(
            spec.observe(&headers(&[("x-release", "  r1\u{7}\t ")]), ""),
            Some("r1".into())
        );
        assert_eq!(spec.observe(&headers(&[("x-release", " \t")]), ""), None);
        let long = "a".repeat(MAX_RELEASE_MARKER_CHARS + 10);
        assert_eq!(
            spec.observe(&headers(&[("x-release", &long)]), "")
                .map(|value| value.chars().count()),
            Some(MAX_RELEASE_MARKER_CHARS)
        );
    }

    #[test]
    fn a_meta_marker_ignores_scripts_and_comments() {
        let spec = ReleaseMarkerSpec::new(None, Some("release"))
            .unwrap()
            .unwrap();
        let html = r#"<!-- <meta name="release" content="old"> -->
            <script>"<meta name='release' content='js'>"</script>
            <meta name="other" content="x"><META NAME="Release" content=" r2 ">"#;
        assert_eq!(spec.observe(&BTreeMap::new(), html), Some("r2".into()));
        assert_eq!(spec.observe(&BTreeMap::new(), "<p>none</p>"), None);
    }

    #[test]
    fn the_header_takes_precedence_and_the_meta_is_its_fallback() {
        let spec = ReleaseMarkerSpec::new(Some("x-release"), Some("release"))
            .unwrap()
            .unwrap();
        let html = r#"<meta name="release" content="from-meta">"#;
        assert_eq!(
            spec.observe(&headers(&[("x-release", "from-header")]), html),
            Some("from-header".into())
        );
        assert_eq!(
            spec.observe(&BTreeMap::new(), html),
            Some("from-meta".into())
        );
    }

    fn artifact(markers: &[Option<&str>]) -> AuditArtifact {
        let pages = markers
            .iter()
            .enumerate()
            .map(|(index, marker)| aexeo_contracts::AuditPageSnapshot {
                route: format!("p{index}"),
                crawl_response: Some(aexeo_contracts::CrawlResponseProvenance {
                    requested_url: format!("https://example.org/p{index}"),
                    release_marker: marker.map(str::to_string),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        AuditArtifact {
            site: Some(aexeo_contracts::AuditSiteSnapshot {
                pages,
                ..Default::default()
            }),
            ..AuditArtifact::default()
        }
    }

    #[test]
    fn a_release_is_attested_only_when_every_page_agrees() {
        let spec = ReleaseMarkerSpec::new(Some("x-release"), None)
            .unwrap()
            .unwrap();
        let agreed = attest_release(&spec, &artifact(&[Some("r1"), Some("r1")]));
        assert_eq!(agreed.value.as_deref(), Some("r1"));
        assert_eq!(agreed.distinct_values, ["r1"]);

        let missing = attest_release(&spec, &artifact(&[Some("r1"), None]));
        assert_eq!(missing.value, None);
        assert_eq!(
            (missing.pages_with_marker, missing.pages_without_marker),
            (1, 1)
        );

        let mixed = attest_release(&spec, &artifact(&[Some("r1"), Some("r2")]));
        assert_eq!(mixed.value, None);
        assert_eq!(mixed.distinct_values, ["r1", "r2"]);
        assert_eq!(mixed.distinct_value_count, None);

        assert_eq!(attest_release(&spec, &artifact(&[])).value, None);
    }

    #[test]
    fn distinct_values_are_capped_with_the_true_count_kept() {
        let spec = ReleaseMarkerSpec::new(None, Some("release"))
            .unwrap()
            .unwrap();
        let values = (0..RELEASE_DISTINCT_VALUES_CAP + 3)
            .map(|index| format!("r{index:02}"))
            .collect::<Vec<_>>();
        let markers = values
            .iter()
            .map(|value| Some(value.as_str()))
            .collect::<Vec<_>>();
        let attested = attest_release(&spec, &artifact(&markers));
        assert_eq!(attested.source, ReleaseMarkerSource::Meta);
        assert_eq!(attested.distinct_values.len(), RELEASE_DISTINCT_VALUES_CAP);
        assert_eq!(
            attested.distinct_value_count,
            Some(RELEASE_DISTINCT_VALUES_CAP + 3)
        );
    }
}
