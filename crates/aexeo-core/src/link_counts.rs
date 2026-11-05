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
    contexts: BTreeMap<LinkContext, BTreeSet<String>>,
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
                destination
                    .contexts
                    .entry(link.context)
                    .or_default()
                    .insert(link.source.clone());
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
                source_pages_by_context: accumulator
                    .contexts
                    .into_iter()
                    .map(|(context, sources)| (context, sources.len()))
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
