//! Explicit candidate snapshots and bounded, coverage-aware link impact.
//!
//! This is an opt-in host interface, not a publication authorization protocol.
//! The host is responsible for the origin, freshness and completeness of the
//! snapshot and for faithfully projecting all relevant rendered links. This
//! module checks internal consistency; it cannot establish those assertions.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use aexeo_contracts::{
    content_gate_change_validation::validate_change_structure,
    content_gate_protocol::{Base, Change, Operation},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Exact host-provided scope at the change's base revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateSnapshot {
    pub scope: String,
    pub site_id: String,
    pub site_base_url: Option<String>,
    pub base: Base,
    /// Host assertion, not independent proof that the inventory is complete.
    pub complete: bool,
    pub items: Vec<CandidateItem>,
}

/// Complete source and host-projected outgoing route links for one entity.
/// `links: None` means unobserved, not that the entity has no links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateItem {
    pub entity_id: String,
    pub public_path: String,
    pub media_type: String,
    pub revision: Option<String>,
    pub sha256: String,
    pub source: String,
    pub links: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateImpact {
    /// Changed identities, including the old identity of a rename.
    pub changed: Vec<String>,
    /// Existing candidate identities directly adjacent to changed routes in
    /// either the old or candidate graph, including changed surviving items.
    pub affected: Vec<String>,
    /// False whenever an inventory or any link projection is unavailable.
    pub link_impact_complete: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateOverlay {
    pub change_id: String,
    pub candidate: CandidateSnapshot,
    pub impact: CandidateImpact,
}

fn digest(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

// Routes are already resolved by the host. Ambiguous URLs, query strings,
// fragments and redirect semantics must not silently become route equality.
fn valid_route(route: &str) -> bool {
    route.starts_with('/')
        && !route.starts_with("//")
        && !route.contains(['?', '#', '\\'])
        && !route.chars().any(char::is_control)
        && !route.split('/').any(|part| part == "." || part == "..")
}

fn validate_item(item: &CandidateItem) -> Result<(), &'static str> {
    if item.entity_id.is_empty()
        || item.media_type.is_empty()
        || !valid_route(&item.public_path)
        || item.revision.as_ref().is_some_and(String::is_empty)
        || item.sha256 != digest(&item.source)
        || item
            .links
            .as_ref()
            .is_some_and(|links| links.iter().any(|link| !valid_route(link)))
    {
        return Err("invalid candidate item");
    }
    Ok(())
}

pub(crate) fn validate_inventory(items: &[CandidateItem]) -> Result<(), &'static str> {
    validate_inventory_inner(items.iter(), true)
}

fn validate_inventory_inner<'a>(
    items: impl ExactSizeIterator<Item = &'a CandidateItem> + Clone,
    validate_items: bool,
) -> Result<(), &'static str> {
    const MAX_ITEMS: usize = 100_000;
    const MAX_LINKS: usize = 1_000_000;
    const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
    const MAX_NON_SOURCE_BYTES: usize = 64 * 1024 * 1024;
    if items.len() > MAX_ITEMS {
        return Err("candidate inventory exceeds item limit");
    }
    let mut source_bytes = 0usize;
    let mut link_count = 0usize;
    let mut non_source_bytes = 0usize;
    for item in items.clone() {
        source_bytes = source_bytes
            .checked_add(item.source.len())
            .ok_or("candidate inventory exceeds source limit")?;
        link_count = link_count
            .checked_add(item.links.as_ref().map_or(0, Vec::len))
            .ok_or("candidate inventory exceeds link limit")?;
        if source_bytes > MAX_SOURCE_BYTES || link_count > MAX_LINKS {
            return Err("candidate inventory exceeds resource limit");
        }
        let lengths = [
            item.entity_id.len(),
            item.public_path.len(),
            item.media_type.len(),
            item.revision.as_ref().map_or(0, String::len),
            item.sha256.len(),
        ];
        for length in lengths
            .into_iter()
            .chain(item.links.iter().flatten().map(String::len))
        {
            non_source_bytes = non_source_bytes
                .checked_add(length)
                .filter(|&total| total <= MAX_NON_SOURCE_BYTES)
                .ok_or("candidate inventory exceeds non-source byte limit")?;
        }
    }
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for item in items {
        if validate_items {
            validate_item(item)?;
        }
        if !ids.insert(&item.entity_id) || !paths.insert(&item.public_path) {
            return Err("duplicate identity or public path");
        }
    }
    Ok(())
}

/// An owned, immutable snapshot whose inventory was validated in this process.
/// This capability is deliberately crate-private and cannot be deserialized.
pub(crate) struct ValidatedCandidateSnapshot(CandidateSnapshot);

impl ValidatedCandidateSnapshot {
    pub(crate) fn new(snapshot: CandidateSnapshot) -> Result<Self, &'static str> {
        validate_inventory(&snapshot.items)?;
        Ok(Self(snapshot))
    }

    pub(crate) fn overlay(
        &self,
        change: &Change,
        after: &[CandidateItem],
    ) -> Result<CandidateOverlay, &'static str> {
        overlay_candidate_inner(&self.0, change, after, true)
    }
}

impl std::ops::Deref for ValidatedCandidateSnapshot {
    type Target = CandidateSnapshot;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Apply a v1 change to a host snapshot and compute one-hop route impact.
///
/// `after` contains exactly one projected item per non-delete entry. Both
/// before and after source bytes are verified against the change digests.
/// Updates, deletes and renames require an explicit before state. Identity,
/// route and revision conflicts are rejected instead of guessed. The returned
/// snapshot retains its *base* revision: it is a candidate, not a published
/// revision. This API does not evaluate rules or prove link extraction coverage.
pub fn overlay_candidate(
    base: &CandidateSnapshot,
    change: &Change,
    after: &[CandidateItem],
) -> Result<CandidateOverlay, &'static str> {
    overlay_candidate_inner(base, change, after, false)
}

fn overlay_candidate_inner(
    base: &CandidateSnapshot,
    change: &Change,
    after: &[CandidateItem],
    base_prevalidated: bool,
) -> Result<CandidateOverlay, &'static str> {
    validate_change_structure(change).map_err(|_| "invalid change")?;
    if base.scope.is_empty() || base.site_id.is_empty() || base.base != change.base {
        return Err("snapshot scope or base revision mismatch");
    }
    if change
        .site
        .as_ref()
        .and_then(|site| site.id.as_ref())
        .is_some_and(|id| id != &base.site_id)
    {
        return Err("snapshot site mismatch");
    }
    if change
        .site
        .as_ref()
        .and_then(|site| site.base_url.as_ref())
        .is_some_and(|url| Some(url) != base.site_base_url.as_ref())
    {
        return Err("snapshot site URL mismatch");
    }
    if !base_prevalidated {
        validate_inventory(&base.items)?;
    }
    validate_inventory(after)?;
    let mut items: BTreeMap<&str, &CandidateItem> = base
        .items
        .iter()
        .map(|item| (item.entity_id.as_str(), item))
        .collect();
    let mut projected: BTreeMap<&str, &CandidateItem> = after
        .iter()
        .map(|item| (item.entity_id.as_str(), item))
        .collect();
    let mut changed = BTreeSet::new();
    let mut routes = BTreeSet::new();
    // Remove all old identities first, enabling route swaps without allowing
    // a change entry to consume an entity twice.
    let mut consumed = BTreeSet::new();
    for entry in &change.entries {
        changed.insert(entry.entity_id.clone());
        let old_id = match entry.operation {
            Operation::Create => continue,
            Operation::Rename => entry
                .previous_entity_id
                .as_deref()
                .ok_or("missing previous identity")?,
            _ => &entry.entity_id,
        };
        if !consumed.insert(old_id.to_string()) {
            return Err("old identity used twice");
        }
        changed.insert(old_id.to_string());
        let old = items.remove(old_id).ok_or("base identity unavailable")?;
        let before = entry.before.as_ref().ok_or("before state unavailable")?;
        if before
            .inline
            .as_ref()
            .is_some_and(|source| source != &old.source)
            || before.sha256 != old.sha256
            || before
                .revision
                .as_ref()
                .is_some_and(|r| Some(r) != old.revision.as_ref())
        {
            return Err("before state mismatch");
        }
        routes.insert(old.public_path.clone());
    }
    for entry in &change.entries {
        if matches!(entry.operation, Operation::Delete) {
            continue;
        }
        let next = projected
            .remove(entry.entity_id.as_str())
            .ok_or("after projection unavailable")?;
        let state = entry.after.as_ref().ok_or("after state unavailable")?;
        if state
            .inline
            .as_ref()
            .is_some_and(|source| source != &next.source)
            || next.sha256 != state.sha256
            || next.media_type != entry.media_type
            || state
                .revision
                .as_ref()
                .is_some_and(|r| Some(r) != next.revision.as_ref())
            || entry
                .public_path
                .as_ref()
                .is_some_and(|path| path != &next.public_path)
        {
            return Err("after projection mismatch");
        }
        if items.insert(entry.entity_id.as_str(), next).is_some() {
            return Err("candidate identity conflict");
        }
        routes.insert(next.public_path.clone());
    }
    if !projected.is_empty() {
        return Err("unexpected after projection");
    }
    // Every candidate item is an unchanged reference to an already validated
    // base or after item. Check the combined limits and route uniqueness before
    // cloning, without hashing every source a second time.
    validate_inventory_inner(items.values().copied(), false)?;
    let candidate_items: Vec<_> = items.into_values().cloned().collect();
    let mut affected = BTreeSet::new();
    let candidate_ids: BTreeSet<&str> = candidate_items
        .iter()
        .map(|item| item.entity_id.as_str())
        .collect();
    for id in &changed {
        if candidate_ids.contains(id.as_str()) {
            affected.insert(id.clone());
        }
    }
    // Include both incoming and outgoing neighbours on either side of the
    // change. A renamed/deleted source can still affect its former targets.
    for inventory in [&base.items[..], &candidate_items[..]] {
        let by_path: BTreeMap<&str, &str> = inventory
            .iter()
            .map(|i| (i.public_path.as_str(), i.entity_id.as_str()))
            .collect();
        for item in inventory {
            if let Some(links) = &item.links {
                if links.iter().any(|link| routes.contains(link))
                    && candidate_ids.contains(item.entity_id.as_str())
                {
                    affected.insert(item.entity_id.clone());
                }
                if routes.contains(&item.public_path) {
                    for link in links {
                        if let Some(id) = by_path.get(link.as_str())
                            && candidate_ids.contains(id)
                        {
                            affected.insert((*id).to_string());
                        }
                    }
                }
            }
        }
    }
    let mut reasons = Vec::new();
    if !base.complete {
        reasons.push("base inventory completeness is not attested".into());
    }
    if base
        .items
        .iter()
        .chain(&candidate_items)
        .any(|item| item.links.is_none())
    {
        reasons.push("one or more link projections are unavailable".into());
    }
    Ok(CandidateOverlay {
        change_id: change.change_id.clone(),
        candidate: CandidateSnapshot {
            scope: base.scope.clone(),
            site_id: base.site_id.clone(),
            site_base_url: base.site_base_url.clone(),
            base: base.base.clone(),
            complete: base.complete,
            items: candidate_items,
        },
        impact: CandidateImpact {
            changed: changed.into_iter().collect(),
            affected: affected.into_iter().collect(),
            link_impact_complete: reasons.is_empty(),
            reasons,
        },
    })
}

/// An optional conservative expansion of direct impact through incoming links.
/// Completeness has exactly the same host-attested limits as `CandidateImpact`;
/// it does not cover dependencies other than projected route links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitiveLinkImpact {
    /// Surviving direct neighbours plus all surviving reverse-reachable sources.
    pub affected: Vec<String>,
    pub link_impact_complete: bool,
    pub reasons: Vec<String>,
}

/// Build the candidate and expand its link impact through reverse dependencies
/// in the union of the old and candidate graphs. Old identities are bridged to
/// new identities for explicit renames only. Cycles terminate and missing links
/// retain the same incomplete status as direct impact. This is a conservative
/// route-link analysis, not a proof of other gate dependencies or freshness.
pub fn overlay_candidate_with_transitive_link_impact(
    base: &CandidateSnapshot,
    change: &Change,
    after: &[CandidateItem],
) -> Result<(CandidateOverlay, TransitiveLinkImpact), &'static str> {
    let overlay = overlay_candidate(base, change, after)?;
    let renames: BTreeMap<&str, &str> = change
        .entries
        .iter()
        .filter_map(|entry| {
            if matches!(entry.operation, Operation::Rename) {
                entry
                    .previous_entity_id
                    .as_deref()
                    .map(|old| (old, entry.entity_id.as_str()))
            } else {
                None
            }
        })
        .collect();
    fn canonical<'a>(id: &'a str, old: bool, renames: &BTreeMap<&'a str, &'a str>) -> &'a str {
        if old {
            renames.get(id).copied().unwrap_or(id)
        } else {
            id
        }
    }
    let mut incoming: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (inventory, old) in [(&base.items, true), (&overlay.candidate.items, false)] {
        let paths: BTreeMap<&str, &str> = inventory
            .iter()
            .map(|item| {
                (
                    item.public_path.as_str(),
                    canonical(&item.entity_id, old, &renames),
                )
            })
            .collect();
        for item in inventory {
            if let Some(links) = &item.links {
                let source = canonical(&item.entity_id, old, &renames);
                for link in links {
                    if let Some(target) = paths.get(link.as_str()) {
                        incoming.entry(target).or_default().insert(source);
                    }
                }
            }
        }
    }
    let mut reached: BTreeSet<&str> = BTreeSet::new();
    let mut pending = VecDeque::new();
    for entry in &change.entries {
        if reached.insert(entry.entity_id.as_str()) {
            pending.push_back(entry.entity_id.as_str());
        }
    }
    while let Some(target) = pending.pop_front() {
        if let Some(sources) = incoming.get(target) {
            for source in sources {
                if reached.insert(source) {
                    pending.push_back(source);
                }
            }
        }
    }
    let mut affected: BTreeSet<String> = overlay.impact.affected.iter().cloned().collect();
    for item in &overlay.candidate.items {
        if reached.contains(item.entity_id.as_str()) {
            affected.insert(item.entity_id.clone());
        }
    }
    let impact = TransitiveLinkImpact {
        affected: affected.into_iter().collect(),
        link_impact_complete: overlay.impact.link_impact_complete,
        reasons: overlay.impact.reasons.clone(),
    };
    Ok((overlay, impact))
}

/// A projected link to a route present in the base but absent from the candidate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemovedRouteReference {
    pub source_entity_id: String,
    pub source_path: String,
    pub destination_path: String,
}

/// Bounded comparison of two host-supplied inventories, not a broken-link verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemovedRouteReferences {
    pub removed_routes: Vec<String>,
    /// Distinct source entity and destination pairs, in deterministic order.
    pub references: Vec<RemovedRouteReference>,
    pub complete: bool,
    pub reasons: Vec<String>,
}

/// Compare projected routes and links. This does not establish that the candidate
/// derives from any change; callers should first use `overlay_candidate`.
/// Completeness relies on host assertions and does not cover redirects, aliases,
/// rendered output, locale fallbacks or other dependencies.
pub fn compare_removed_route_references(
    base: &CandidateSnapshot,
    candidate: &CandidateSnapshot,
) -> Result<RemovedRouteReferences, &'static str> {
    if base.scope.is_empty()
        || base.site_id.is_empty()
        || base.base.revision.is_empty()
        || base.scope != candidate.scope
        || base.site_id != candidate.site_id
        || base.site_base_url != candidate.site_base_url
        || base.base != candidate.base
    {
        return Err("snapshot scope or base revision mismatch");
    }
    validate_inventory(&base.items)?;
    validate_inventory(&candidate.items)?;
    let candidate_routes: BTreeSet<&str> = candidate
        .items
        .iter()
        .map(|item| item.public_path.as_str())
        .collect();
    let removed: BTreeSet<&str> = base
        .items
        .iter()
        .map(|item| item.public_path.as_str())
        .filter(|path| !candidate_routes.contains(path))
        .collect();
    const MAX_REFERENCES: usize = 10_000;
    const MAX_REFERENCE_BYTES: usize = 8 * 1024 * 1024;
    // Keep the sorted prefix plus one sentinel for truncation. A full set can
    // otherwise grow to the million-link input limit before applying the cap.
    let mut references = BTreeSet::new();
    for item in &candidate.items {
        if let Some(links) = &item.links {
            for destination in links {
                if removed.contains(destination.as_str()) {
                    let reference = (
                        item.entity_id.as_str(),
                        item.public_path.as_str(),
                        destination.as_str(),
                    );
                    if references.len() <= MAX_REFERENCES
                        || references.last().is_some_and(|last| reference < *last)
                    {
                        references.insert(reference);
                        if references.len() > MAX_REFERENCES + 1 {
                            references.pop_last();
                        }
                    }
                }
            }
        }
    }
    let mut reasons = Vec::new();
    if !base.complete || !candidate.complete {
        reasons.push("inventory completeness is not attested".into());
    }
    if candidate.items.iter().any(|item| item.links.is_none()) {
        reasons.push("one or more candidate link projections are unavailable".into());
    }
    // Borrow while deduplicating to avoid multiplying large path strings in memory.
    // Bound the owned result as well: truncation can never certify completeness.
    let mut output = Vec::new();
    let mut output_bytes = 0usize;
    for (source_entity_id, source_path, destination_path) in references {
        let size = source_entity_id
            .len()
            .saturating_add(source_path.len())
            .saturating_add(destination_path.len());
        if output.len() == MAX_REFERENCES || size > MAX_REFERENCE_BYTES.saturating_sub(output_bytes)
        {
            reasons.push("removed-route reference output limit exceeded".into());
            break;
        }
        output_bytes += size;
        output.push(RemovedRouteReference {
            source_entity_id: source_entity_id.into(),
            source_path: source_path.into(),
            destination_path: destination_path.into(),
        });
    }
    Ok(RemovedRouteReferences {
        removed_routes: removed.into_iter().map(str::to_owned).collect(),
        references: output,
        complete: reasons.is_empty(),
        reasons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aexeo_contracts::content_gate_protocol::{
        BaseKind, ContentState, Entry, Origin, OriginKind,
    };

    fn item(id: &str, route: &str, source: &str, links: Option<Vec<&str>>) -> CandidateItem {
        CandidateItem {
            entity_id: id.into(),
            public_path: route.into(),
            media_type: "text/plain".into(),
            revision: Some("r1".into()),
            sha256: digest(source),
            source: source.into(),
            links: links.map(|v| v.into_iter().map(str::to_owned).collect()),
        }
    }
    fn state(value: &CandidateItem) -> ContentState {
        ContentState {
            revision: value.revision.clone(),
            sha256: value.sha256.clone(),
            inline: Some(value.source.clone()),
            file: None,
            uri: None,
        }
    }
    fn setup() -> (CandidateSnapshot, Change) {
        let base = Base {
            kind: BaseKind::Opaque,
            revision: "base".into(),
        };
        let snapshot = CandidateSnapshot {
            scope: "published".into(),
            site_id: "example".into(),
            site_base_url: None,
            base: base.clone(),
            complete: true,
            items: vec![
                item("a", "/a", "A", Some(vec!["/b"])),
                item("b", "/b", "B", Some(vec![])),
                item("c", "/c", "C", Some(vec!["/a"])),
            ],
        };
        let change = Change {
            schema: "content-gate.change/v1".into(),
            change_id: "change".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            origin: Origin {
                kind: OriginKind::Manual,
                system: None,
                reference: None,
            },
            base,
            content_root: None,
            site: None,
            author: None,
            entries: vec![],
            extensions: None,
        };
        (snapshot, change)
    }
    fn entry(
        id: &str,
        operation: Operation,
        before: Option<&CandidateItem>,
        after: Option<&CandidateItem>,
    ) -> Entry {
        Entry {
            entity_id: id.into(),
            operation,
            previous_entity_id: None,
            media_type: "text/plain".into(),
            public_path: after.map(|i| i.public_path.clone()),
            metadata: None,
            before: before.map(state),
            after: after.map(state),
        }
    }

    #[test]
    fn combined_candidate_rejects_excessive_link_bytes_before_overlay() {
        let (mut base, mut change) = setup();
        base.items.truncate(1);
        let large_link = format!("/{}", "x".repeat(33 * 1024 * 1024));
        base.items[0].links = Some(vec![large_link.clone()]);
        let mut created = item("new", "/new", "new", Some(vec![]));
        created.links = Some(vec![large_link]);
        assert!(validate_inventory(&base.items).is_ok());
        assert!(validate_inventory(std::slice::from_ref(&created)).is_ok());
        change
            .entries
            .push(entry("new", Operation::Create, None, Some(&created)));
        assert!(matches!(
            overlay_candidate(&base, &change, &[created]),
            Err("candidate inventory exceeds non-source byte limit")
        ));
    }

    #[test]
    fn removed_route_references_cover_changes_and_uncertainty() {
        let (mut base, mut change) = setup();
        base.items[2].links = Some(vec!["/a".into(), "/a".into(), "/missing".into()]);
        let renamed = item("renamed", "/new", "new", Some(vec![]));
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&base.items[0]),
            Some(&renamed),
        );
        rename.previous_entity_id = Some("a".into());
        change.entries.push(rename);
        let overlay = overlay_candidate(&base, &change, &[renamed]).unwrap();
        let report = compare_removed_route_references(&base, &overlay.candidate).unwrap();
        assert!(report.complete);
        assert_eq!(report.removed_routes, ["/a"]);
        assert_eq!(
            report.references,
            [RemovedRouteReference {
                source_entity_id: "c".into(),
                source_path: "/c".into(),
                destination_path: "/a".into()
            }]
        );

        let mut incomplete = overlay.candidate.clone();
        incomplete
            .items
            .iter_mut()
            .find(|item| item.entity_id == "b")
            .unwrap()
            .links = None;
        let report = compare_removed_route_references(&base, &incomplete).unwrap();
        assert!(!report.complete);
        assert_eq!(report.references.len(), 1);
        assert!(!report.reasons.is_empty());

        incomplete.complete = false;
        assert!(
            !compare_removed_route_references(&base, &incomplete)
                .unwrap()
                .complete
        );
        incomplete.base.revision = "other".into();
        assert!(compare_removed_route_references(&base, &incomplete).is_err());
    }

    #[test]
    fn removed_route_reference_limit_keeps_the_sorted_prefix() {
        let (mut base, _) = setup();
        base.items = vec![item("removed", "/removed", "old", Some(vec![]))];
        let mut candidate = base.clone();
        candidate.items = (0..10_100)
            .rev()
            .map(|i| {
                item(
                    &format!("source-{i:05}"),
                    &format!("/source-{i:05}"),
                    "x",
                    Some(vec!["/removed", "/removed"]),
                )
            })
            .collect();
        let report = compare_removed_route_references(&base, &candidate).unwrap();
        assert!(!report.complete);
        let expected: Vec<_> = (0..10_000)
            .map(|i| RemovedRouteReference {
                source_entity_id: format!("source-{i:05}"),
                source_path: format!("/source-{i:05}"),
                destination_path: "/removed".into(),
            })
            .collect();
        assert_eq!(report.references, expected);
        assert!(
            report
                .reasons
                .iter()
                .any(|reason| reason.contains("output limit"))
        );
        candidate.items.truncate(10_000);
        let report = compare_removed_route_references(&base, &candidate).unwrap();
        assert!(report.complete);
        assert_eq!(report.references.len(), 10_000);
    }

    #[test]
    fn removed_route_reference_byte_limit_is_incomplete() {
        let (mut base, mut change) = setup();
        base.items[2].entity_id = "x".repeat(8 * 1024 * 1024);
        change
            .entries
            .push(entry("a", Operation::Delete, Some(&base.items[0]), None));
        let candidate = overlay_candidate(&base, &change, &[]).unwrap().candidate;
        let report = compare_removed_route_references(&base, &candidate).unwrap();
        assert!(!report.complete);
        assert!(report.references.is_empty());
        assert!(
            report
                .reasons
                .iter()
                .any(|reason| reason.contains("output limit"))
        );
    }

    #[test]
    fn reused_route_is_not_reported_as_removed() {
        let (base, mut change) = setup();
        let replacement = item("replacement", "/a", "new", Some(vec![]));
        change
            .entries
            .push(entry("a", Operation::Delete, Some(&base.items[0]), None));
        change.entries.push(entry(
            "replacement",
            Operation::Create,
            None,
            Some(&replacement),
        ));
        let overlay = overlay_candidate(&base, &change, &[replacement]).unwrap();
        let report = compare_removed_route_references(&base, &overlay.candidate).unwrap();
        assert!(report.complete);
        assert!(report.removed_routes.is_empty());
        assert!(report.references.is_empty());
    }

    #[test]
    fn rename_tracks_both_graphs_and_returns_complete_candidate() {
        let (snapshot, mut change) = setup();
        let replacement = item("renamed", "/new", "new", Some(vec!["/c"]));
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&snapshot.items[0]),
            Some(&replacement),
        );
        rename.previous_entity_id = Some("a".into());
        change.entries.push(rename);
        let result = overlay_candidate(&snapshot, &change, &[replacement]).unwrap();
        assert_eq!(result.impact.changed, ["a", "renamed"]);
        assert_eq!(result.impact.affected, ["b", "c", "renamed"]);
        assert!(result.impact.link_impact_complete);
        assert_eq!(
            result
                .candidate
                .items
                .iter()
                .map(|i| i.entity_id.as_str())
                .collect::<Vec<_>>(),
            ["b", "c", "renamed"]
        );
    }

    #[test]
    fn delete_and_create_handle_unknown_coverage_without_silent_pass() {
        let (mut snapshot, mut change) = setup();
        snapshot.items[1].links = None;
        change.entries.push(entry(
            "a",
            Operation::Delete,
            Some(&snapshot.items[0]),
            None,
        ));
        let new = item("d", "/d", "D", Some(vec![]));
        change
            .entries
            .push(entry("d", Operation::Create, None, Some(&new)));
        let result = overlay_candidate(&snapshot, &change, &[new]).unwrap();
        assert_eq!(result.impact.affected, ["b", "c", "d"]);
        assert!(!result.impact.link_impact_complete);
        assert_eq!(result.impact.reasons.len(), 1);
    }

    #[test]
    fn transitive_impact_walks_old_and_new_incoming_graphs_with_cycles() {
        let (mut snapshot, mut change) = setup();
        snapshot.items.push(item("d", "/d", "D", Some(vec!["/c"])));
        snapshot.items.push(item("e", "/e", "E", Some(vec!["/d"])));
        snapshot.items[2].links = Some(vec!["/a".into(), "/e".into()]);
        let replacement = item("renamed", "/new", "new", Some(vec![]));
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&snapshot.items[0]),
            Some(&replacement),
        );
        rename.previous_entity_id = Some("a".into());
        change.entries.push(rename);
        let (overlay, impact) =
            overlay_candidate_with_transitive_link_impact(&snapshot, &change, &[replacement])
                .unwrap();
        assert_eq!(overlay.impact.affected, ["b", "c", "renamed"]);
        assert_eq!(impact.affected, ["b", "c", "d", "e", "renamed"]);
        assert!(impact.link_impact_complete);
    }

    #[test]
    fn transitive_impact_includes_candidate_only_edges() {
        let (mut snapshot, mut change) = setup();
        snapshot.items[2].links = Some(vec!["/b".into()]);
        let a = item("a", "/a", "new A", Some(vec![]));
        let b = item("b", "/b", "new B", Some(vec!["/a"]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&a),
        ));
        change.entries.push(entry(
            "b",
            Operation::Update,
            Some(&snapshot.items[1]),
            Some(&b),
        ));
        let (_, impact) =
            overlay_candidate_with_transitive_link_impact(&snapshot, &change, &[a, b]).unwrap();
        assert_eq!(impact.affected, ["a", "b", "c"]);
        assert!(impact.link_impact_complete);
    }

    #[test]
    fn transitive_impact_matches_exhaustive_three_node_graphs() {
        let names = ["a", "b", "c"];
        let paths = ["/a", "/b", "/c"];
        for mask in 0usize..(1 << 9) {
            let (mut snapshot, mut change) = setup();
            for source in 0..3 {
                snapshot.items[source].links = Some(
                    (0..3)
                        .filter(|target| mask & (1 << (source * 3 + target)) != 0)
                        .map(|target| paths[target].to_string())
                        .collect(),
                );
            }
            let replacement = item(
                "a",
                "/a",
                "new",
                Some(
                    (0..3)
                        .filter(|target| mask & (1 << target) != 0)
                        .map(|target| paths[target])
                        .collect(),
                ),
            );
            change.entries.push(entry(
                "a",
                Operation::Update,
                Some(&snapshot.items[0]),
                Some(&replacement),
            ));
            let (_, impact) =
                overlay_candidate_with_transitive_link_impact(&snapshot, &change, &[replacement])
                    .unwrap();
            let mut reverse = [true, false, false];
            loop {
                let mut next = reverse;
                for (source, result) in next.iter_mut().enumerate() {
                    for (target, reachable) in reverse.iter().enumerate() {
                        if *reachable && mask & (1 << (source * 3 + target)) != 0 {
                            *result = true;
                        }
                    }
                }
                if next == reverse {
                    break;
                }
                reverse = next;
            }
            for (target, reachable) in reverse.iter_mut().enumerate() {
                if mask & (1 << target) != 0 {
                    *reachable = true;
                }
            }
            let expected: Vec<String> = (0..3)
                .filter(|i| reverse[*i])
                .map(|i| names[i].to_string())
                .collect();
            assert_eq!(impact.affected, expected, "graph mask {mask}");
            assert!(impact.link_impact_complete);
        }
    }

    #[test]
    fn transitive_impact_is_incomplete_when_link_coverage_is_unknown() {
        let (mut snapshot, mut change) = setup();
        snapshot.items[1].links = None;
        change.entries.push(entry(
            "a",
            Operation::Delete,
            Some(&snapshot.items[0]),
            None,
        ));
        let (_, impact) =
            overlay_candidate_with_transitive_link_impact(&snapshot, &change, &[]).unwrap();
        assert!(!impact.link_impact_complete);
        assert!(!impact.reasons.is_empty());
        assert_eq!(impact.affected, ["b", "c"]);
    }

    #[test]
    fn validated_snapshot_matches_full_overlay_and_still_rejects_bad_projections() {
        let (snapshot, mut change) = setup();
        let after = item("a", "/a", "replacement", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&after),
        ));
        let validated = ValidatedCandidateSnapshot::new(snapshot.clone()).unwrap();
        assert_eq!(
            validated.overlay(&change, std::slice::from_ref(&after)),
            overlay_candidate(&snapshot, &change, std::slice::from_ref(&after)),
        );
        let mut bad_after = after.clone();
        bad_after.source.push_str("tampered");
        assert!(
            validated
                .overlay(&change, std::slice::from_ref(&bad_after))
                .is_err()
        );
        assert_eq!(
            validated.overlay(&change, &[bad_after.clone()]),
            overlay_candidate(&snapshot, &change, &[bad_after]),
        );
        let mut bad_base = snapshot.clone();
        bad_base.items[0].source.push_str("tampered");
        assert!(ValidatedCandidateSnapshot::new(bad_base).is_err());
        let mut collision = after;
        collision.public_path = snapshot.items[1].public_path.clone();
        change.entries[0].public_path = Some(collision.public_path.clone());
        assert_eq!(
            validated.overlay(&change, std::slice::from_ref(&collision)),
            overlay_candidate(&snapshot, &change, std::slice::from_ref(&collision)),
        );
        assert!(validated.overlay(&change, &[collision]).is_err());
    }

    #[test]
    fn rejects_stale_or_inconsistent_inputs_and_collisions() {
        let (snapshot, mut change) = setup();
        let after = item("a", "/b", "new", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&after),
        ));
        assert_eq!(
            overlay_candidate(&snapshot, &change, std::slice::from_ref(&after)).unwrap_err(),
            "duplicate identity or public path"
        );
        let mut other = snapshot.clone();
        other.base.revision = "stale".into();
        assert_eq!(
            overlay_candidate(&other, &change, std::slice::from_ref(&after)).unwrap_err(),
            "snapshot scope or base revision mismatch"
        );
        other = snapshot.clone();
        other.items[0].source = "tampered".into();
        assert_eq!(
            overlay_candidate(&other, &change, std::slice::from_ref(&after)).unwrap_err(),
            "invalid candidate item"
        );
        change.entries[0].after.as_mut().unwrap().inline = Some("different".into());
        assert_eq!(
            overlay_candidate(&snapshot, &change, std::slice::from_ref(&after)).unwrap_err(),
            "after projection mismatch"
        );
        change.entries[0].after.as_mut().unwrap().inline = Some(after.source.clone());
        change.entries[0].before = None;
        assert_eq!(
            overlay_candidate(&snapshot, &change, &[after]).unwrap_err(),
            "before state unavailable"
        );
    }

    #[test]
    fn validates_even_removed_items_and_new_projections() {
        let (mut snapshot, mut change) = setup();
        let after = item("a", "/a", "replacement", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&after),
        ));
        snapshot.items[0].source = "tampered old source".into();
        assert_eq!(
            overlay_candidate(&snapshot, &change, std::slice::from_ref(&after)).unwrap_err(),
            "invalid candidate item"
        );
        snapshot.items[0].source = "A".into();
        let mut tampered_after = after;
        tampered_after.source = "tampered replacement".into();
        assert_eq!(
            overlay_candidate(&snapshot, &change, &[tampered_after]).unwrap_err(),
            "invalid candidate item"
        );
    }

    #[test]
    fn incomplete_inventory_and_unresolved_routes_are_explicit() {
        let (mut snapshot, mut change) = setup();
        snapshot.complete = false;
        change.entries.push(entry(
            "a",
            Operation::Delete,
            Some(&snapshot.items[0]),
            None,
        ));
        assert!(
            !overlay_candidate(&snapshot, &change, &[])
                .unwrap()
                .impact
                .link_impact_complete
        );
        snapshot.items[2].links = Some(vec!["https://example.test/a".into()]);
        assert_eq!(
            overlay_candidate(&snapshot, &change, &[]).unwrap_err(),
            "invalid candidate item"
        );
    }
}
