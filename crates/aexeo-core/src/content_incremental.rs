//! In-process incremental evaluation with a deliberately narrow proof scope.
//!
//! The opaque baseline owns the exact preset and snapshot evaluated by this
//! process. It cannot be deserialized or supplied as an arbitrary cache.
use std::collections::{BTreeMap, BTreeSet};

use aexeo_contracts::content_gate_protocol::Change;
use serde::Serialize;

use crate::{
    content_candidate::{CandidateItem, CandidateSnapshot, overlay_candidate, validate_inventory},
    content_gate::{ContentGate, ContentInput, ContentPreset, ContentReport, GateStatus},
};

/// An unavailable guarantee; this is not a passing result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotChecked {
    pub reason: String,
}

fn unavailable(reason: impl Into<String>) -> NotChecked {
    NotChecked {
        reason: reason.into(),
    }
}

/// Reports evaluated by this binary from an owned base and owned preset.
/// Construct once and reuse for multiple proposals against the same base.
/// The cache is intentionally not serializable and has no mutable accessors.
pub struct IncrementalBaseline {
    snapshot: CandidateSnapshot,
    preset: ContentPreset,
    reports: BTreeMap<String, ContentReport>,
}

/// A complete report only for the narrowly specified independent item gates.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IncrementalEvaluation {
    Evaluated {
        change_id: String,
        reports: BTreeMap<String, ContentReport>,
        /// Number of candidate entities re-evaluated in this request.
        evaluated: usize,
        /// Number of unchanged candidate reports reused from the baseline.
        reused: usize,
    },
    NotChecked {
        reason: String,
    },
}

fn validate_scope(snapshot: &CandidateSnapshot) -> Result<(), NotChecked> {
    if snapshot.scope.is_empty() || snapshot.site_id.is_empty() || snapshot.base.revision.is_empty()
    {
        return Err(unavailable("snapshot scope or base is unavailable"));
    }
    if !snapshot.complete {
        return Err(unavailable("snapshot completeness is not attested"));
    }
    validate_inventory(&snapshot.items).map_err(unavailable)?;
    if snapshot.items.iter().any(|item| item.links.is_none()) {
        return Err(unavailable("link projection is unavailable"));
    }
    if snapshot
        .items
        .iter()
        .any(|item| item.media_type != "text/plain")
    {
        return Err(unavailable(
            "only text/plain source projection is supported",
        ));
    }
    Ok(())
}

fn validate_preset(preset: &ContentPreset) -> Result<(), NotChecked> {
    preset
        .validate()
        .map_err(|_| unavailable("invalid content preset"))?;
    if preset.gates.iter().any(|gate| {
        !matches!(
            gate,
            ContentGate::WordCount { .. }
                | ContentGate::CharacterCount { .. }
                | ContentGate::SentenceLength {
                    paragraphs_only: false,
                    ..
                }
                | ContentGate::SentenceCharacterLength {
                    paragraphs_only: false,
                    ..
                }
                | ContentGate::RepeatedTerms { .. }
                | ContentGate::RequiredTerms { .. }
                | ContentGate::ForbiddenTerms { .. }
        )
    }) {
        return Err(unavailable(
            "preset contains gates outside the supported independent item scope",
        ));
    }
    Ok(())
}

fn evaluate_item(
    preset: &ContentPreset,
    item: &CandidateItem,
) -> Result<ContentReport, NotChecked> {
    let input = ContentInput {
        id: item.entity_id.clone(),
        text: Some(item.source.clone()),
        ..Default::default()
    };
    let report = preset
        .evaluate(&input)
        .map_err(|_| unavailable("content evaluation failed"))?;
    if report
        .gates
        .iter()
        .any(|gate| gate.status == GateStatus::NotChecked)
    {
        return Err(unavailable("a required item gate was not checked"));
    }
    Ok(report)
}

impl IncrementalBaseline {
    /// Evaluate all base entities once. A host's completeness and link
    /// projection claims are prerequisites, not independently proven facts.
    pub fn prepare(snapshot: CandidateSnapshot, preset: ContentPreset) -> Result<Self, NotChecked> {
        validate_scope(&snapshot)?;
        validate_preset(&preset)?;
        let mut reports = BTreeMap::new();
        for item in &snapshot.items {
            reports.insert(item.entity_id.clone(), evaluate_item(&preset, item)?);
        }
        Ok(Self {
            snapshot,
            preset,
            reports,
        })
    }

    /// Evaluate a proposal against the *same* owned base and preset. The
    /// overlay and impact are recomputed, not accepted from the caller.
    /// All surviving directly affected entities are re-evaluated; other
    /// independent-item reports are reused. Site and collection gates are
    /// never inferred from this result. Errors fail closed as `not_checked`.
    pub fn evaluate(&self, change: &Change, after: &[CandidateItem]) -> IncrementalEvaluation {
        match self.evaluate_checked(change, after) {
            Ok(result) => result,
            Err(error) => IncrementalEvaluation::NotChecked {
                reason: error.reason,
            },
        }
    }

    fn evaluate_checked(
        &self,
        change: &Change,
        after: &[CandidateItem],
    ) -> Result<IncrementalEvaluation, NotChecked> {
        if change
            .entries
            .iter()
            .any(|entry| entry.media_type != "text/plain")
        {
            return Err(unavailable("only text/plain change entries are supported"));
        }
        if change
            .entries
            .iter()
            .flat_map(|entry| [entry.before.as_ref(), entry.after.as_ref()])
            .flatten()
            .any(|state| state.inline.is_none())
        {
            return Err(unavailable("only inline change content is supported"));
        }
        let overlay = overlay_candidate(&self.snapshot, change, after).map_err(unavailable)?;
        if !overlay.impact.link_impact_complete {
            return Err(unavailable("link impact coverage is incomplete"));
        }
        validate_scope(&overlay.candidate)?;
        let affected: BTreeSet<&str> = overlay.impact.affected.iter().map(String::as_str).collect();
        let baseline_items: BTreeMap<&str, &CandidateItem> = self
            .snapshot
            .items
            .iter()
            .map(|item| (item.entity_id.as_str(), item))
            .collect();
        let mut reports = BTreeMap::new();
        let mut evaluated = 0;
        let mut reused = 0;
        for item in &overlay.candidate.items {
            let report = if affected.contains(item.entity_id.as_str()) {
                evaluated += 1;
                evaluate_item(&self.preset, item)?
            } else {
                if baseline_items.get(item.entity_id.as_str()).copied() != Some(item) {
                    return Err(unavailable(
                        "baseline item differs from candidate outside the impact set",
                    ));
                }
                reused += 1;
                self.reports
                    .get(&item.entity_id)
                    .ok_or_else(|| unavailable("baseline report is unavailable"))?
                    .clone()
            };
            reports.insert(item.entity_id.clone(), report);
        }
        Ok(IncrementalEvaluation::Evaluated {
            change_id: change.change_id.clone(),
            reports,
            evaluated,
            reused,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aexeo_contracts::content_gate_protocol::{
        Base, BaseKind, ContentState, Entry, Operation, Origin, OriginKind,
    };
    use sha2::{Digest, Sha256};

    fn item(id: &str, source: &str, links: Option<Vec<&str>>) -> CandidateItem {
        CandidateItem {
            entity_id: id.into(),
            public_path: format!("/{id}"),
            media_type: "text/plain".into(),
            revision: Some("r1".into()),
            sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
            source: source.into(),
            links: links.map(|v| v.into_iter().map(str::to_owned).collect()),
        }
    }
    fn state(item: &CandidateItem) -> ContentState {
        ContentState {
            revision: item.revision.clone(),
            sha256: item.sha256.clone(),
            inline: Some(item.source.clone()),
            file: None,
            uri: None,
        }
    }
    fn entry(
        id: &str,
        op: Operation,
        before: Option<&CandidateItem>,
        after: Option<&CandidateItem>,
    ) -> Entry {
        Entry {
            entity_id: id.into(),
            operation: op,
            previous_entity_id: None,
            media_type: "text/plain".into(),
            public_path: after.map(|item| item.public_path.clone()),
            metadata: None,
            before: before.map(state),
            after: after.map(state),
        }
    }
    fn fixture() -> (CandidateSnapshot, ContentPreset, Change) {
        let base = Base {
            kind: BaseKind::Opaque,
            revision: "base".into(),
        };
        let snapshot = CandidateSnapshot {
            scope: "published".into(),
            site_id: "test".into(),
            site_base_url: None,
            base: base.clone(),
            complete: true,
            items: vec![
                item("a", "one", Some(vec!["/b"])),
                item("b", "two words", Some(vec![])),
                item("c", "three different words", Some(vec![])),
                item("d", "four rather different words", Some(vec!["/a"])),
                item("e", "untouched", Some(vec![])),
            ],
        };
        let preset=ContentPreset::from_toml("version=1\nname='checks'\n[[gates]]\nid='words'\nkind='word_count'\nmin=2\n[[gates]]\nid='chars'\nkind='character_count'\nmin=4\n").unwrap();
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
        (snapshot, preset, change)
    }
    fn full(
        snapshot: &CandidateSnapshot,
        preset: &ContentPreset,
    ) -> BTreeMap<String, ContentReport> {
        snapshot
            .items
            .iter()
            .map(|item| {
                let input = ContentInput {
                    id: item.entity_id.clone(),
                    text: Some(item.source.clone()),
                    ..Default::default()
                };
                (item.entity_id.clone(), preset.evaluate(&input).unwrap())
            })
            .collect()
    }
    #[test]
    fn incremental_equals_full_after_update_delete_create_and_rename() {
        let (snapshot, preset, mut change) = fixture();
        let updated = item("a", "new source words", Some(vec!["/c"]));
        let created = item("new", "created item", Some(vec![]));
        let renamed = item("renamed", "renamed entry", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&updated),
        ));
        change.entries.push(entry(
            "b",
            Operation::Delete,
            Some(&snapshot.items[1]),
            None,
        ));
        change
            .entries
            .push(entry("new", Operation::Create, None, Some(&created)));
        let mut rename = entry(
            "renamed",
            Operation::Rename,
            Some(&snapshot.items[2]),
            Some(&renamed),
        );
        rename.previous_entity_id = Some("c".into());
        change.entries.push(rename);
        let after = [updated, created, renamed];
        let overlay = overlay_candidate(&snapshot, &change, &after).unwrap();
        let expected = full(&overlay.candidate, &preset);
        let baseline = IncrementalBaseline::prepare(snapshot, preset).unwrap();
        for _ in 0..2 {
            let IncrementalEvaluation::Evaluated {
                reports,
                evaluated,
                reused,
                ..
            } = baseline.evaluate(&change, &after)
            else {
                panic!("expected evaluated")
            };
            assert_eq!(
                serde_json::to_value(&reports).unwrap(),
                serde_json::to_value(&expected).unwrap()
            );
            assert_eq!(evaluated + reused, reports.len());
            assert!(reused > 0);
        }
    }
    #[test]
    fn text_only_gates_match_full_evaluation_for_each_gate() {
        for gate in [
            "kind='sentence_length'\nmax_words=2\nabbreviations=['Dr.']",
            "kind='sentence_character_length'\nmax_chars=8\ndelimiters='.!?'",
            "kind='repeated_terms'\nterms=['café']\nmax_occurrences=1\ncase_insensitive=true",
            "kind='required_terms'\nterms=['café', 'absent']\ncase_insensitive=true",
            "kind='forbidden_terms'\nterms=['café']\ncase_insensitive=true",
        ] {
            let (snapshot, _, mut change) = fixture();
            let preset = ContentPreset::from_toml(&format!(
                "version=1\nname='text'\n[[gates]]\nid='check'\n{gate}\n"
            ))
            .unwrap();
            let updated = item(
                "a",
                "Dr. Café CAFÉ. Other words here!\r\nNext sentence.",
                Some(vec![]),
            );
            change.entries.push(entry(
                "a",
                Operation::Update,
                Some(&snapshot.items[0]),
                Some(&updated),
            ));
            let after = [updated];
            let candidate = overlay_candidate(&snapshot, &change, &after)
                .unwrap()
                .candidate;
            let expected = full(&candidate, &preset);
            let baseline = IncrementalBaseline::prepare(snapshot, preset).unwrap();
            let IncrementalEvaluation::Evaluated {
                reports,
                evaluated,
                reused,
                ..
            } = baseline.evaluate(&change, &after)
            else {
                panic!("gate did not evaluate: {gate}");
            };
            assert_eq!(
                serde_json::to_value(&reports).unwrap(),
                serde_json::to_value(&expected).unwrap(),
                "{gate}"
            );
            assert_eq!(evaluated + reused, candidate.items.len());
            assert!(evaluated > 0 && reused > 0, "{gate}");
        }
    }
    #[test]
    fn incomplete_dependencies_and_unsupported_scope_are_never_passed() {
        let (mut snapshot, preset, mut change) = fixture();
        snapshot.complete = false;
        assert!(IncrementalBaseline::prepare(snapshot.clone(), preset.clone()).is_err());
        snapshot.complete = true;
        snapshot.items[0].links = None;
        assert!(IncrementalBaseline::prepare(snapshot.clone(), preset.clone()).is_err());
        snapshot.items[0].links = Some(vec![]);
        for gate in [
            "kind='sentence_length'\nmax_words=2\nparagraphs_only=true",
            "kind='sentence_character_length'\nmax_chars=8\nparagraphs_only=true",
            "kind='metadata_length'\nfield='title'\nmin=1",
            "kind='unique_collection_metadata'\nfield='title'",
            "kind='site_audit'",
            "kind='custom'\nextension='remote'",
        ] {
            let source = format!("version=1\nname='unsupported'\n[[gates]]\nid='other'\n{gate}\n");
            let unsupported = ContentPreset::from_toml(&source).unwrap();
            assert!(IncrementalBaseline::prepare(snapshot.clone(), unsupported).is_err());
        }
        let mut next = item("a", "new text", None);
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        ));
        let baseline = IncrementalBaseline::prepare(snapshot.clone(), preset).unwrap();
        let mut deletion = change.clone();
        deletion.entries = vec![entry(
            "b",
            Operation::Delete,
            Some(&snapshot.items[1]),
            None,
        )];
        deletion.entries[0].media_type = "text/html".into();
        assert!(matches!(
            baseline.evaluate(&deletion, &[]),
            IncrementalEvaluation::NotChecked { .. }
        ));
        assert!(matches!(
            baseline.evaluate(&change, std::slice::from_ref(&next)),
            IncrementalEvaluation::NotChecked { .. }
        ));
        next.links = Some(vec![]);
        next.media_type = "text/html".into();
        change.entries[0].media_type = "text/html".into();
        assert!(matches!(
            baseline.evaluate(&change, &[next]),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }
    #[test]
    fn stale_or_tampered_proposal_is_not_checked() {
        let (snapshot, preset, mut change) = fixture();
        let next = item("a", "new text", Some(vec![]));
        change.entries.push(entry(
            "a",
            Operation::Update,
            Some(&snapshot.items[0]),
            Some(&next),
        ));
        let baseline = IncrementalBaseline::prepare(snapshot, preset).unwrap();
        change.base.revision = "stale".into();
        assert!(matches!(
            baseline.evaluate(&change, std::slice::from_ref(&next)),
            IncrementalEvaluation::NotChecked { .. }
        ));
        change.base.revision = "base".into();
        change.entries[0].after.as_mut().unwrap().inline = None;
        change.entries[0].after.as_mut().unwrap().uri = Some("https://example.test/source".into());
        assert!(matches!(
            baseline.evaluate(&change, std::slice::from_ref(&next)),
            IncrementalEvaluation::NotChecked { .. }
        ));
        change.entries[0].after.as_mut().unwrap().uri = None;
        change.entries[0].after.as_mut().unwrap().inline = Some("tampered".into());
        assert!(matches!(
            baseline.evaluate(&change, &[next]),
            IncrementalEvaluation::NotChecked { .. }
        ));
    }
}
