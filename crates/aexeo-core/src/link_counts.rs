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
    /// Sources outside known conditional, hidden or collapsed markup (not proof of visibility).
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

/// A host-owned, source-keyed snapshot for recalculating only destinations
/// touched by page changes. Source order is lexical and each source retains
/// its original evidence order, matching `count_links` on that flattened input.
/// The host must supply complete, current evidence for each source; this type
/// does not establish inventory completeness, freshness, redirects or visibility.
pub struct IncrementalLinkCounts {
    sources: BTreeMap<String, Vec<LinkEvidence>>,
    incoming: BTreeMap<String, BTreeSet<String>>,
}

impl IncrementalLinkCounts {
    /// Empty evidence is an observed page with no links, not an omitted page.
    pub fn new(sources: BTreeMap<String, Vec<LinkEvidence>>) -> Result<Self, &'static str> {
        let mut incoming = BTreeMap::<String, BTreeSet<String>>::new();
        for (source, evidence) in &sources {
            validate_source(source, evidence)?;
            for link in evidence {
                if let LinkResolution::Internal { url } = &link.resolution {
                    incoming
                        .entry(url.clone())
                        .or_default()
                        .insert(source.clone());
                }
            }
        }
        Ok(Self { sources, incoming })
    }

    /// Replace or remove whole source observations without changing the base.
    /// Each touched destination maps to its new count, or `None` when no
    /// observations remain. Same-document jumps never become incoming links.
    /// Results are advisory observed counts, not orphan or publication verdicts.
    pub fn evaluate(
        &self,
        changes: &BTreeMap<String, Option<Vec<LinkEvidence>>>,
    ) -> Result<BTreeMap<String, Option<DestinationLinks>>, &'static str> {
        let mut affected = BTreeSet::new();
        for (source, replacement) in changes {
            if source.is_empty() {
                return Err("source must not be empty");
            }
            if replacement.is_none() && !self.sources.contains_key(source) {
                return Err("cannot remove an unobserved source");
            }
            if let Some(evidence) = replacement {
                validate_source(source, evidence)?;
            }
            for link in self
                .sources
                .get(source)
                .into_iter()
                .flatten()
                .chain(replacement.iter().flatten())
            {
                if let LinkResolution::Internal { url } = &link.resolution {
                    affected.insert(url.clone());
                }
            }
        }
        let mut candidate_sources = BTreeSet::new();
        for destination in &affected {
            if let Some(sources) = self.incoming.get(destination) {
                candidate_sources.extend(sources.iter().cloned());
            }
        }
        candidate_sources.extend(changes.keys().cloned());
        let mut evidence = Vec::new();
        for source in candidate_sources {
            let observed = match changes.get(&source) {
                Some(replacement) => replacement.as_ref(),
                None => self.sources.get(&source),
            };
            if let Some(observed) = observed {
                evidence.extend(observed.iter().filter(|link| {
                    matches!(&link.resolution, LinkResolution::Internal { url } if affected.contains(url))
                }).cloned());
            }
        }
        let mut result: BTreeMap<_, _> = affected.into_iter().map(|url| (url, None)).collect();
        for destination in count_links(evidence).destinations {
            result.insert(destination.destination.clone(), Some(destination));
        }
        Ok(result)
    }
}

fn validate_source(source: &str, evidence: &[LinkEvidence]) -> Result<(), &'static str> {
    if source.is_empty() || evidence.iter().any(|link| link.source != source) {
        return Err("link evidence source does not match its inventory key");
    }
    Ok(())
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

    #[test]
    fn incremental_destinations_match_full_recount_after_mixed_changes() {
        fn page(source: &str, html: &str) -> Vec<LinkEvidence> {
            extract_links(html, source, "https://example.org", None)
        }
        let sources = BTreeMap::from([
            (
                "https://example.org/a".into(),
                page(
                    "https://example.org/a",
                    "<nav><a href='/one'>one</a></nav><main><a href='/one'>again</a><a href='#x'>jump</a></main>",
                ),
            ),
            (
                "https://example.org/b".into(),
                page(
                    "https://example.org/b",
                    "<main><a href='/one'>one</a><a href='/two'>two</a></main>",
                ),
            ),
            (
                "https://example.org/c".into(),
                page(
                    "https://example.org/c",
                    "<footer><a href='/two'>two</a></footer>",
                ),
            ),
            (
                "https://example.org/unrelated".into(),
                page(
                    "https://example.org/unrelated",
                    "<main><a href='/unrelated-target'>other</a></main>",
                ),
            ),
        ]);
        let baseline = IncrementalLinkCounts::new(sources.clone()).unwrap();
        let changes = BTreeMap::from([
            (
                "https://example.org/a".into(),
                Some(page(
                    "https://example.org/a",
                    "<main><a href='/two'>new</a><a href='/three'>third</a></main>",
                )),
            ),
            ("https://example.org/b".into(), None),
            (
                "https://example.org/d".into(),
                Some(page(
                    "https://example.org/d",
                    "<nav><a href='/two'>menu</a></nav><main><a href='/three'>body</a></main>",
                )),
            ),
        ]);
        let actual = baseline.evaluate(&changes).unwrap();
        let mut candidate = sources;
        for (source, replacement) in &changes {
            match replacement {
                Some(evidence) => {
                    candidate.insert(source.clone(), evidence.clone());
                }
                None => {
                    candidate.remove(source);
                }
            }
        }
        let full = count_links(candidate.into_values().flatten().collect());
        for url in [
            "https://example.org/one",
            "https://example.org/two",
            "https://example.org/three",
        ] {
            assert_eq!(
                actual[url].as_ref(),
                full.destinations.iter().find(|d| d.destination == url)
            );
        }
        assert_eq!(actual.len(), 3);
        assert!(actual["https://example.org/one"].is_none());
        assert_eq!(
            actual["https://example.org/two"]
                .as_ref()
                .unwrap()
                .distinct_source_pages,
            3
        );
        assert!(!actual.contains_key("https://example.org/unrelated-target"));
        assert!(baseline.evaluate(&BTreeMap::new()).unwrap().is_empty());
    }

    #[test]
    fn incremental_rejects_misattributed_evidence_and_unknown_deletions() {
        let evidence = extract_links(
            "<a href='/x'>x</a>",
            "https://example.org/a",
            "https://example.org",
            None,
        );
        assert!(
            IncrementalLinkCounts::new(BTreeMap::from([("wrong".into(), evidence.clone())]))
                .is_err()
        );
        let baseline = IncrementalLinkCounts::new(BTreeMap::from([(
            "https://example.org/a".into(),
            evidence.clone(),
        )]))
        .unwrap();
        assert!(
            baseline
                .evaluate(&BTreeMap::from([("other".into(), Some(evidence))]))
                .is_err()
        );
        assert!(
            baseline
                .evaluate(&BTreeMap::from([("missing".into(), None)]))
                .is_err()
        );
        assert!(
            baseline
                .evaluate(&BTreeMap::from([(
                    "https://example.org/a".into(),
                    Some(Vec::new())
                )]))
                .unwrap()["https://example.org/x"]
                .is_none()
        );
    }
}
