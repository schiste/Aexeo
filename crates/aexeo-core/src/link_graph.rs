//! Building blocks for a coverage-aware link graph.
//!
//! Resolution requires the observed page URL and an explicit site origin.
//! Missing context is never interpreted as proof that a link is external.
use serde::{Deserialize, Serialize};
use url::Url;

/// Classification of a link using an explicit page URL and site origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkResolution {
    /// A fragment or other reference to exactly the same path and query.
    SameDocument { url: String },
    /// An HTTP(S) URL on the configured origin, with its query preserved.
    Internal { url: String },
    /// An HTTP(S) URL whose origin differs from the configured origin.
    External { url: String },
    /// A non-HTTP destination, such as mailto or tel.
    Unsupported { scheme: String },
    /// The supplied inputs do not permit a reliable classification.
    Indeterminate { reason: String },
}

/// Resolve a reference using the exact observed page URL and explicit site origin.
///
/// Origins include scheme, host and effective port; aliases and redirects are
/// deliberately not inferred. A caller handling HTML `<base href>` must supply
/// its resolved effective base separately. URLs retain query strings but drop
/// fragments, since fragments do not select another HTTP document.
pub fn resolve_link(
    href: &str,
    page_url: &str,
    site_origin: &str,
    effective_base: Option<&str>,
) -> LinkResolution {
    fn valid_http(value: &str) -> Option<Url> {
        let url = Url::parse(value).ok()?;
        (matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none())
        .then_some(url)
    }
    let Some(page) = valid_http(page_url) else {
        return LinkResolution::Indeterminate {
            reason: "page URL is missing or invalid".into(),
        };
    };
    let Some(site) = valid_http(site_origin) else {
        return LinkResolution::Indeterminate {
            reason: "site origin is missing or invalid".into(),
        };
    };
    if page.origin() != site.origin() {
        return LinkResolution::Indeterminate {
            reason: "page URL does not match the site origin".into(),
        };
    }
    let base = match effective_base {
        Some(value) => match page.join(value) {
            Ok(url)
                if matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none() =>
            {
                url
            }
            _ => {
                return LinkResolution::Indeterminate {
                    reason: "effective base URL is invalid".into(),
                };
            }
        },
        None => page.clone(),
    };
    let mut target = match base.join(href) {
        Ok(url) => url,
        Err(_) => {
            return LinkResolution::Indeterminate {
                reason: "link URL is invalid".into(),
            };
        }
    };
    if !matches!(target.scheme(), "http" | "https") {
        return LinkResolution::Unsupported {
            scheme: target.scheme().into(),
        };
    }
    if !target.username().is_empty() || target.password().is_some() {
        return LinkResolution::Indeterminate {
            reason: "link URL contains credentials".into(),
        };
    }
    target.set_fragment(None);
    let mut source = page;
    source.set_fragment(None);
    if target == source {
        LinkResolution::SameDocument { url: target.into() }
    } else if target.origin() == site.origin() {
        LinkResolution::Internal { url: target.into() }
    } else {
        LinkResolution::External { url: target.into() }
    }
}

fn comparable_host(host: &str) -> String {
    let without_userinfo = host.rsplit_once('@').map(|(_, rest)| rest).unwrap_or(host);
    let without_port = if without_userinfo.starts_with('[') {
        without_userinfo
            .split_once(']')
            .map(|(value, _)| value)
            .unwrap_or(without_userinfo)
    } else {
        without_userinfo
            .split_once(':')
            .map(|(value, _)| value)
            .unwrap_or(without_userinfo)
    };
    let normalized = without_port.trim_end_matches('.').to_ascii_lowercase();
    normalized
        .strip_prefix("www.")
        .unwrap_or(&normalized)
        .to_string()
}

pub(crate) fn same_site_host(left: &str, right: &str) -> bool {
    comparable_host(left) == comparable_host(right)
}

/// A URL safe to record in findings and artifacts, which may be shared. Userinfo,
/// query and fragment are dropped because they can carry credentials and tokens.
/// Routes never include a query, so this loses nothing the audit depends on.
pub fn redact_url(url: &str) -> String {
    // `user:pass@host` parses as a URL with the scheme `user`, so the scheme matters.
    let Some(mut parsed) = Url::parse(url)
        .ok()
        .filter(|parsed| matches!(parsed.scheme(), "http" | "https"))
    else {
        return "[invalid URL]".to_string();
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_urls_keep_only_scheme_host_port_and_path() {
        assert_eq!(
            redact_url("https://user:pass@example.org:8443/a/b?token=x#frag"),
            "https://example.org:8443/a/b"
        );
        assert_eq!(redact_url("user:pass@example.org"), "[invalid URL]");
        assert_eq!(redact_url("not a url with secret"), "[invalid URL]");
    }
    fn resolve(href: &str) -> LinkResolution {
        resolve_link(
            href,
            "https://example.org/fr/guide/",
            "https://example.org",
            None,
        )
    }
    #[test]
    fn resolves_references_without_guessing_origins() {
        assert_eq!(
            resolve("#intro"),
            LinkResolution::SameDocument {
                url: "https://example.org/fr/guide/".into()
            }
        );
        assert_eq!(
            resolve("../article?lang=fr#intro"),
            LinkResolution::Internal {
                url: "https://example.org/fr/article?lang=fr".into()
            }
        );
        assert_eq!(
            resolve("https://example.org/about"),
            LinkResolution::Internal {
                url: "https://example.org/about".into()
            }
        );
        assert_eq!(
            resolve("//example.org:443/about"),
            LinkResolution::Internal {
                url: "https://example.org/about".into()
            }
        );
        assert_eq!(
            resolve("https://example.org:444/about"),
            LinkResolution::External {
                url: "https://example.org:444/about".into()
            }
        );
        assert_eq!(
            resolve("https://example.org.evil.test/about"),
            LinkResolution::External {
                url: "https://example.org.evil.test/about".into()
            }
        );
        assert_eq!(
            resolve("mailto:a@example.org"),
            LinkResolution::Unsupported {
                scheme: "mailto".into()
            }
        );
        assert!(matches!(
            resolve("https://user:password@example.org/"),
            LinkResolution::Indeterminate { .. }
        ));
    }
    #[test]
    fn bases_queries_and_missing_context_remain_explicit() {
        assert_eq!(
            resolve("?page=2"),
            LinkResolution::Internal {
                url: "https://example.org/fr/guide/?page=2".into()
            }
        );
        assert_eq!(
            resolve_link(
                "item",
                "https://example.org/fr/guide/",
                "https://example.org",
                Some("/docs/")
            ),
            LinkResolution::Internal {
                url: "https://example.org/docs/item".into()
            }
        );
        assert!(matches!(
            resolve_link("/a", "https://example.org/", "", None),
            LinkResolution::Indeterminate { .. }
        ));
        assert!(matches!(
            resolve_link("/a", "https://example.org/", "https://other.test", None),
            LinkResolution::Indeterminate { .. }
        ));
        assert!(matches!(
            resolve_link(
                "/a",
                "https://example.org/",
                "https://example.org",
                Some("javascript:alert(1)")
            ),
            LinkResolution::Indeterminate { .. }
        ));
    }
}
