//! Count distinct source pages per link context without inferring orphanhood.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::link_evidence::{LinkContext, LinkEvidence};
use crate::link_graph::LinkResolution;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationLinks {
    pub destination: String,
    pub distinct_source_pages: usize,
    pub source_pages_by_context: BTreeMap<LinkContext, usize>,
    /// Sources with conditional, hidden or legacy unclassified markup; may overlap other counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditional_source_pages: Option<usize>,
    /// Sources outside conditional markup and without an HTML hidden attribute (not proof of visibility).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonconditional_source_pages: Option<usize>,
    /// Source pages with at least one link carrying no `nofollow`, `ugc` or
    /// `sponsored` token. Zero with sources present means every link is qualified.
    #[serde(default)]
    pub followed_source_pages: usize,
    /// Source pages with at least one link carrying each `rel` token.
    #[serde(default)]
    pub source_pages_by_rel: BTreeMap<String, usize>,
    /// Individual observations retain anchor, source, component and locale.
    pub evidence: Vec<LinkEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkCounts {
    pub destinations: Vec<DestinationLinks>,
    pub same_document: Vec<LinkEvidence>,
    pub external: Vec<LinkEvidence>,
    pub unsupported: Vec<LinkEvidence>,
    pub indeterminate: Vec<LinkEvidence>,
}

#[derive(Default)]
struct Accumulator {
    sources: BTreeSet<String>,
    conditional: BTreeSet<String>,
    nonconditional: BTreeSet<String>,
    contexts: BTreeMap<LinkContext, BTreeSet<String>>,
    followed: BTreeSet<String>,
    rel: BTreeMap<String, BTreeSet<String>>,
    evidence: Vec<LinkEvidence>,
}

/// Count exactly observed source URLs. Query variants remain distinct and
/// redirects, canonical aliases and locale fallbacks are never inferred here.
/// Absence from this report is not evidence that a page is an orphan.
pub fn count_links(evidence: Vec<LinkEvidence>) -> LinkCounts {
    let mut destinations = BTreeMap::<String, Accumulator>::new();
    let mut same_document = Vec::new();
    let mut external = Vec::new();
    let mut unsupported = Vec::new();
    let mut indeterminate = Vec::new();
    for link in evidence {
        match &link.resolution {
            LinkResolution::Internal { url } => {
                let destination = destinations.entry(url.clone()).or_default();
                destination.sources.insert(link.source.clone());
                if link.is_nonconditional() {
                    destination.nonconditional.insert(link.source.clone());
                } else {
                    destination.conditional.insert(link.source.clone());
                }
                destination
                    .contexts
                    .entry(link.context)
                    .or_default()
                    .insert(link.source.clone());
                if link.rel.is_empty() {
                    destination.followed.insert(link.source.clone());
                }
                for token in &link.rel {
                    destination
                        .rel
                        .entry(token.clone())
                        .or_default()
                        .insert(link.source.clone());
                }
                destination.evidence.push(link);
            }
            LinkResolution::SameDocument { .. } => same_document.push(link),
            LinkResolution::External { .. } => external.push(link),
            LinkResolution::Unsupported { .. } => unsupported.push(link),
            LinkResolution::Indeterminate { .. } => indeterminate.push(link),
        }
    }
    LinkCounts {
        destinations: destinations
            .into_iter()
            .map(|(destination, accumulator)| DestinationLinks {
                destination,
                distinct_source_pages: accumulator.sources.len(),
                conditional_source_pages: Some(accumulator.conditional.len()),
                nonconditional_source_pages: Some(accumulator.nonconditional.len()),
                source_pages_by_context: accumulator
                    .contexts
                    .into_iter()
                    .map(|(context, sources)| (context, sources.len()))
                    .collect(),
                followed_source_pages: accumulator.followed.len(),
                source_pages_by_rel: accumulator
                    .rel
                    .into_iter()
                    .map(|(token, sources)| (token, sources.len()))
                    .collect(),
                evidence: accumulator.evidence,
            })
            .collect(),
        same_document,
        external,
        unsupported,
        indeterminate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link_evidence::extract_links;

    #[test]
    fn counts_distinct_sources_per_category_and_keeps_jumps_separate() {
        let html = "<nav><a href='/target'>menu</a></nav><main><p><a href='/target'>one</a><a href='https://example.org/target'>two</a><a href='#part'>jump</a></p></main>";
        let mut links = extract_links(
            html,
            "https://example.org/source",
            "https://example.org",
            None,
        );
        links.extend(extract_links(
            "<main><a href='/target'>three</a></main>",
            "https://example.org/other",
            "https://example.org",
            None,
        ));
        let result = count_links(links);
        assert_eq!(result.destinations.len(), 1);
        let target = &result.destinations[0];
        assert_eq!(target.destination, "https://example.org/target");
        assert_eq!(target.distinct_source_pages, 2);
        assert_eq!(target.source_pages_by_context[&LinkContext::Body], 2);
        assert_eq!(target.source_pages_by_context[&LinkContext::Navigation], 1);
        assert_eq!(target.evidence.len(), 4);
        assert_eq!(result.same_document.len(), 1);
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(
            json["destinations"][0]["source_pages_by_context"]["body"],
            2
        );
    }

    #[test]
    fn conditional_markup_does_not_inflate_body_or_footer_counts() {
        let links = extract_links(
            "<main><noscript><a href='/target'>fallback</a></noscript><a href='/target'>body</a></main><footer><template><a href='/target'>template</a></template></footer>",
            "https://example.org/source",
            "https://example.org",
            None,
        );
        let counts = count_links(links);
        let target = &counts.destinations[0];
        assert_eq!(target.distinct_source_pages, 1);
        assert_eq!(target.conditional_source_pages, Some(1));
        assert_eq!(target.nonconditional_source_pages, Some(1));
        assert_eq!(target.source_pages_by_context[&LinkContext::Body], 1);
        assert_eq!(target.source_pages_by_context[&LinkContext::Unknown], 1);
        assert!(
            !target
                .source_pages_by_context
                .contains_key(&LinkContext::Footer)
        );
        assert_eq!(target.evidence.len(), 3);
    }

    #[test]
    fn preserves_uncertain_and_non_http_links() {
        let links = extract_links(
            "<a href='/a'>a</a><a href='mailto:a@example.org'>email</a>",
            "not a valid URL",
            "https://example.org",
            None,
        );
        let result = count_links(links);
        assert!(result.destinations.is_empty());
        assert_eq!(result.indeterminate.len(), 2);
        let links = extract_links(
            "<a href='mailto:a@example.org'>email</a><a href='https://other.example/'>other</a>",
            "https://example.org/",
            "https://example.org",
            None,
        );
        let result = count_links(links);
        assert_eq!(result.unsupported.len(), 1);
        assert_eq!(result.external.len(), 1);
    }
}
