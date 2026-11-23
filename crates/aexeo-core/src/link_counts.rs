//! Count distinct source pages per link context without inferring orphanhood.
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

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
    sources: BTreeSet<Rc<str>>,
    conditional: BTreeSet<Rc<str>>,
    nonconditional: BTreeSet<Rc<str>>,
    contexts: BTreeMap<LinkContext, BTreeSet<Rc<str>>>,
    followed: BTreeSet<Rc<str>>,
    rel: BTreeMap<String, BTreeSet<Rc<str>>>,
    evidence: Vec<LinkEvidence>,
}

/// Count exactly observed source URLs. Query variants remain distinct and
/// redirects, canonical aliases and locale fallbacks are never inferred here.
/// Absence from this report is not evidence that a page is an orphan.
pub fn count_links(evidence: Vec<LinkEvidence>) -> LinkCounts {
    let mut destinations = BTreeMap::<String, Accumulator>::new();
    // Each observed source can occur in many destinations and count sets.
    // Share its allocation while retaining the original evidence unchanged.
    let mut source_pool = BTreeSet::<Rc<str>>::new();
    let mut last_source: Option<Rc<str>> = None;
    let mut same_document = Vec::new();
    let mut external = Vec::new();
    let mut unsupported = Vec::new();
    let mut indeterminate = Vec::new();
    for link in evidence {
        match &link.resolution {
            LinkResolution::Internal { url } => {
                if last_source.as_deref() != Some(link.source.as_str()) {
                    let source = match source_pool.get(link.source.as_str()) {
                        Some(source) => Rc::clone(source),
                        None => {
                            let source: Rc<str> = Rc::from(link.source.as_str());
                            source_pool.insert(Rc::clone(&source));
                            source
                        }
                    };
                    last_source = Some(source);
                }
                let source = last_source.as_ref().expect("source initialized above");
                let destination = destinations.entry(url.clone()).or_default();
                destination.sources.insert(Rc::clone(source));
                if link.is_nonconditional() {
                    destination.nonconditional.insert(Rc::clone(source));
                } else {
                    destination.conditional.insert(Rc::clone(source));
                }
                destination
                    .contexts
                    .entry(link.context)
                    .or_default()
                    .insert(Rc::clone(source));
                if link.rel.is_empty() {
                    destination.followed.insert(Rc::clone(source));
                }
                for token in &link.rel {
                    destination
                        .rel
                        .entry(token.clone())
                        .or_default()
                        .insert(Rc::clone(source));
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

/// Destination changes and aggregate work counters for one candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncrementalLinkCountsReport {
    pub destinations: BTreeMap<String, Option<DestinationLinks>>,
    /// Candidate source observations actually scanned to recount destinations.
    pub sources_scanned: usize,
    /// Source observations in the candidate inventory, including empty pages.
    pub candidate_sources: usize,
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
    ) -> Result<IncrementalLinkCountsReport, &'static str> {
        let mut affected = BTreeSet::new();
        let mut replacement_sources_with_links = BTreeSet::new();
        let mut candidate_count = self.sources.len();
        for (source, replacement) in changes {
            if source.is_empty() {
                return Err("source must not be empty");
            }
            if replacement.is_none() && !self.sources.contains_key(source) {
                return Err("cannot remove an unobserved source");
            }
            if let Some(evidence) = replacement {
                validate_source(source, evidence)?;
                if !self.sources.contains_key(source) {
                    candidate_count += 1;
                }
            } else {
                candidate_count -= 1;
            }
            for link in self.sources.get(source).into_iter().flatten() {
                if let LinkResolution::Internal { url } = &link.resolution {
                    affected.insert(url.clone());
                }
            }
            let mut replacement_has_internal = false;
            for link in replacement.iter().flatten() {
                if let LinkResolution::Internal { url } = &link.resolution {
                    affected.insert(url.clone());
                    replacement_has_internal = true;
                }
            }
            if replacement_has_internal {
                replacement_sources_with_links.insert(source.clone());
            }
        }
        let mut candidate_sources = BTreeSet::new();
        for destination in &affected {
            if let Some(sources) = self.incoming.get(destination) {
                candidate_sources.extend(sources.iter().cloned());
            }
        }
        candidate_sources.extend(replacement_sources_with_links);
        let mut evidence = Vec::new();
        let mut sources_scanned = 0;
        for source in candidate_sources {
            let observed = match changes.get(&source) {
                Some(replacement) => replacement.as_ref(),
                None => self.sources.get(&source),
            };
            if let Some(observed) = observed {
                sources_scanned += 1;
                evidence.extend(observed.iter().filter(|link| {
                    matches!(&link.resolution, LinkResolution::Internal { url } if affected.contains(url))
                }).cloned());
            }
        }
        let mut result: BTreeMap<_, _> = affected.into_iter().map(|url| (url, None)).collect();
        for destination in count_links(evidence).destinations {
            result.insert(destination.destination.clone(), Some(destination));
        }
        Ok(IncrementalLinkCountsReport {
            destinations: result,
            sources_scanned,
            candidate_sources: candidate_count,
        })
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
    fn repeated_sources_keep_distinct_counts_across_rel_and_contexts() {
        let mut links = extract_links(
            "<nav><a href='/target' rel='nofollow'>menu</a></nav><main><a href='/target'>one</a><a href='/target' rel='nofollow ugc'>two</a><a href='/other'>other</a></main>",
            "https://example.org/source",
            "https://example.org",
            None,
        );
        links.extend(extract_links(
            "<main><a href='/target' rel='ugc'>three</a></main>",
            "https://example.org/second",
            "https://example.org",
            None,
        ));
        let result = count_links(links);
        let target = result
            .destinations
            .iter()
            .find(|d| d.destination.ends_with("/target"))
            .unwrap();
        assert_eq!(target.distinct_source_pages, 2);
        assert_eq!(target.source_pages_by_context[&LinkContext::Body], 2);
        assert_eq!(target.source_pages_by_context[&LinkContext::Navigation], 1);
        assert_eq!(target.followed_source_pages, 1);
        assert_eq!(target.source_pages_by_rel["nofollow"], 1);
        assert_eq!(target.source_pages_by_rel["ugc"], 2);
        assert_eq!(target.evidence.len(), 4);
    }

    #[test]
    fn incremental_bulk_additions_do_not_recount_sources_without_internal_links() {
        let mut changes = BTreeMap::new();
        for n in 0..1000 {
            changes.insert(format!("https://example.org/empty-{n}"), Some(Vec::new()));
        }
        let external_source = "https://example.org/external-only";
        changes.insert(
            external_source.into(),
            Some(extract_links(
                "<a href='https://outside.example/'>external</a>",
                external_source,
                "https://example.org",
                None,
            )),
        );
        let source = "https://example.org/new";
        let links = extract_links(
            "<main><a href='/target'>target</a></main>",
            source,
            "https://example.org",
            None,
        );
        changes.insert(source.into(), Some(links.clone()));
        let report = IncrementalLinkCounts::new(BTreeMap::new())
            .unwrap()
            .evaluate(&changes)
            .unwrap();
        let full = count_links(links);
        assert_eq!(report.candidate_sources, 1002);
        assert_eq!(
            report.destinations["https://example.org/target"],
            Some(full.destinations[0].clone())
        );
        assert_eq!(report.sources_scanned, 1);
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
        let report = baseline.evaluate(&changes).unwrap();
        let actual = report.destinations;
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
        assert_eq!(report.sources_scanned, 3);
        assert_eq!(report.candidate_sources, 4);
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
        assert!(
            baseline
                .evaluate(&BTreeMap::new())
                .unwrap()
                .destinations
                .is_empty()
        );
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
        let empty = baseline
            .evaluate(&BTreeMap::from([(
                "https://example.org/empty".into(),
                Some(Vec::new()),
            )]))
            .unwrap();
        assert!(empty.destinations.is_empty());
        assert_eq!(empty.sources_scanned, 0);
        assert_eq!(empty.candidate_sources, 2);
        assert!(
            baseline
                .evaluate(&BTreeMap::from([(
                    "https://example.org/a".into(),
                    Some(Vec::new())
                )]))
                .unwrap()
                .destinations["https://example.org/x"]
                .is_none()
        );
    }

    #[test]
    fn incremental_matches_recount_across_source_change_combinations() {
        let names = [
            "https://example.org/a",
            "https://example.org/b",
            "https://example.org/c",
        ];
        let original = |source: &str| {
            extract_links(
                "<nav><a href='/one'>one</a></nav><main><a href='/two' rel='nofollow'>two</a></main>",
                source,
                "https://example.org",
                None,
            )
        };
        let replacement = |source: &str| {
            extract_links(
                "<main><a href='/two'>two</a><a href='/three'>three</a><a href='#jump'>jump</a></main>",
                source,
                "https://example.org",
                None,
            )
        };
        let sources: BTreeMap<_, _> = names
            .iter()
            .map(|source| ((*source).to_string(), original(source)))
            .collect();
        let baseline = IncrementalLinkCounts::new(sources.clone()).unwrap();
        for mask in 0..64 {
            let mut candidate = sources.clone();
            let mut changes = BTreeMap::new();
            for (index, source) in names.iter().enumerate() {
                let value = match (mask >> (index * 2)) & 3 {
                    0 => continue,
                    1 => None,
                    2 => Some(Vec::new()),
                    _ => Some(replacement(source)),
                };
                match &value {
                    Some(evidence) => {
                        candidate.insert((*source).into(), evidence.clone());
                    }
                    None => {
                        candidate.remove(*source);
                    }
                }
                changes.insert((*source).into(), value);
            }
            let actual = baseline.evaluate(&changes).unwrap().destinations;
            let full = count_links(candidate.into_values().flatten().collect());
            for (url, destination) in &actual {
                assert_eq!(
                    destination.as_ref(),
                    full.destinations
                        .iter()
                        .find(|item| &item.destination == url),
                    "mask {mask}, {url}"
                );
            }
            if mask != 0 {
                assert!(actual.contains_key("https://example.org/one"));
                assert!(actual.contains_key("https://example.org/two"));
            }
            assert_eq!(
                actual.contains_key("https://example.org/three"),
                names
                    .iter()
                    .enumerate()
                    .any(|(index, _)| ((mask >> (index * 2)) & 3) == 3)
            );
        }
    }
}
