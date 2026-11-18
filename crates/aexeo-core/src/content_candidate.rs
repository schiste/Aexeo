//! Explicit candidate snapshots and bounded, coverage-aware link impact.
//!
//! This is an opt-in host interface, not a publication authorization protocol.
//! The host is responsible for the origin, freshness and completeness of the
//! snapshot and for faithfully projecting all relevant rendered links. This
//! module checks internal consistency; it cannot establish those assertions.
use std::collections::{BTreeMap, BTreeSet};

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

fn validate_inventory(items: &[CandidateItem]) -> Result<(), &'static str> {
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
        if before.sha256 != old.sha256
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
        if next.sha256 != state.sha256
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
            overlay_candidate(&snapshot, &change, &[after.clone()]).unwrap_err(),
            "duplicate identity or public path"
        );
        let mut other = snapshot.clone();
        other.base.revision = "stale".into();
        assert_eq!(
            overlay_candidate(&other, &change, &[after.clone()]).unwrap_err(),
            "snapshot scope or base revision mismatch"
        );
        other = snapshot.clone();
        other.items[0].source = "tampered".into();
        assert_eq!(
            overlay_candidate(&other, &change, &[after.clone()]).unwrap_err(),
            "invalid candidate item"
        );
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
