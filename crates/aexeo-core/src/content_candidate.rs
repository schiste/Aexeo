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
    const MAX_ITEMS: usize = 100_000;
    const MAX_LINKS: usize = 1_000_000;
    const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
    if items.len() > MAX_ITEMS {
        return Err("candidate inventory exceeds item limit");
    }
    let mut source_bytes = 0usize;
    let mut link_count = 0usize;
    for item in items {
        source_bytes = source_bytes
            .checked_add(item.source.len())
            .ok_or("candidate inventory exceeds source limit")?;
        link_count = link_count
            .checked_add(item.links.as_ref().map_or(0, Vec::len))
            .ok_or("candidate inventory exceeds link limit")?;
        if source_bytes > MAX_SOURCE_BYTES || link_count > MAX_LINKS {
            return Err("candidate inventory exceeds resource limit");
        }
    }
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for item in items {
        validate_item(item)?;
        if !ids.insert(&item.entity_id) || !paths.insert(&item.public_path) {
            return Err("duplicate identity or public path");
        }
    }
    Ok(())
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
    validate_inventory(&base.items)?;
    validate_inventory(after)?;
    let mut items: BTreeMap<String, CandidateItem> = base
        .items
        .iter()
        .cloned()
        .map(|item| (item.entity_id.clone(), item))
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
        routes.insert(old.public_path);
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
        if items
            .insert(entry.entity_id.clone(), next.clone())
            .is_some()
        {
            return Err("candidate identity conflict");
        }
        routes.insert(next.public_path.clone());
    }
    if !projected.is_empty() {
        return Err("unexpected after projection");
    }
    let candidate_items: Vec<_> = items.into_values().collect();
    validate_inventory(&candidate_items)?;
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
            items: candidate_items,
            ..base.clone()
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
                for source in 0..3 {
                    for target in 0..3 {
                        if reverse[target] && mask & (1 << (source * 3 + target)) != 0 {
                            next[source] = true;
                        }
                    }
                }
                if next == reverse {
                    break;
                }
                reverse = next;
            }
            for target in 0..3 {
                if mask & (1 << target) != 0 {
                    reverse[target] = true;
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
