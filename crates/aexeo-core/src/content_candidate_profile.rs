//! Portable experimental identity for the exact fields of a candidate snapshot.
//!
//! A digest is not proof of inventory completeness, freshness or provenance.
use crate::content_candidate::{CandidateItem, CandidateSnapshot};
use aexeo_contracts::content_gate_protocol::BaseKind;
use aexeo_contracts::content_gate_revision_set::{RevisionRecord, revision_set_digest};

const MAX_BYTES: usize = 64 * 1024 * 1024;

fn append(out: &mut Vec<u8>, value: &[u8]) -> Result<(), &'static str> {
    if out
        .len()
        .checked_add(value.len())
        .is_none_or(|len| len > MAX_BYTES)
    {
        return Err("candidate profile exceeds size limit");
    }
    out.extend_from_slice(value);
    Ok(())
}
fn string(out: &mut Vec<u8>, value: &str) -> Result<(), &'static str> {
    append(out, &(value.len() as u64).to_be_bytes())?;
    append(out, value.as_bytes())
}
fn optional(out: &mut Vec<u8>, value: Option<&str>) -> Result<(), &'static str> {
    match value {
        None => append(out, &[0]),
        Some(value) => {
            append(out, &[1])?;
            string(out, value)
        }
    }
}

/// Hash every field represented by `CandidateSnapshot` using the published
/// candidate profile. Caller-supplied inventory and projections remain claims;
/// no publication authority or independently observed completeness is implied.
pub fn candidate_revision_digest(snapshot: &CandidateSnapshot) -> Result<[u8; 32], &'static str> {
    let CandidateSnapshot {
        scope: _,
        site_id: _,
        site_base_url: _,
        base: _,
        complete: _,
        items: _,
    } = snapshot;
    let mut scope = b"content-gate:candidate-scope:v1\0".to_vec();
    string(&mut scope, &snapshot.scope)?;
    string(&mut scope, &snapshot.site_id)?;
    optional(&mut scope, snapshot.site_base_url.as_deref())?;
    let mut descriptor = b"content-gate:candidate-snapshot:v1\0".to_vec();
    string(
        &mut descriptor,
        match &snapshot.base.kind {
            BaseKind::Git => "git",
            BaseKind::Opaque => "opaque",
        },
    )?;
    string(&mut descriptor, &snapshot.base.revision)?;
    append(&mut descriptor, &[u8::from(snapshot.complete)])?;
    if snapshot.items.len() > 100_000 {
        return Err("too many candidate items");
    }
    let mut records = Vec::with_capacity(snapshot.items.len());
    let mut total = scope.len() + descriptor.len();
    for item in &snapshot.items {
        let CandidateItem {
            entity_id: _,
            public_path: _,
            media_type: _,
            revision: _,
            sha256: _,
            source: _,
            links: _,
        } = item;
        let mut record = b"content-gate:candidate-item:v1\0".to_vec();
        string(&mut record, &item.public_path)?;
        string(&mut record, &item.media_type)?;
        optional(&mut record, item.revision.as_deref())?;
        string(&mut record, &item.sha256)?;
        string(&mut record, &item.source)?;
        match &item.links {
            None => append(&mut record, &[0])?,
            Some(links) => {
                append(&mut record, &[1])?;
                let count = u32::try_from(links.len()).map_err(|_| "too many candidate links")?;
                append(&mut record, &count.to_be_bytes())?;
                for link in links {
                    string(&mut record, link)?;
                }
            }
        }
        total = total
            .checked_add(item.entity_id.len())
            .and_then(|n| n.checked_add(record.len()))
            .ok_or("candidate profile exceeds size limit")?;
        if total > MAX_BYTES {
            return Err("candidate profile exceeds size limit");
        }
        records.push(record);
    }
    let entries: Vec<_> = snapshot
        .items
        .iter()
        .zip(&records)
        .map(|(item, bytes)| RevisionRecord {
            entity_id: &item.entity_id,
            record: bytes,
        })
        .collect();
    revision_set_digest(&scope, &descriptor, &entries)
}

/// Bind explicit decision inputs to the exact candidate snapshot, recomputing
/// its revision digest rather than accepting a digest from a previous verdict.
/// The caller must independently obtain the current snapshot and complete
/// decision inputs. This API does not authenticate the snapshot or authorize
/// publication and does not imply a complete or fresh inventory.
pub fn candidate_decision_binding(
    snapshot: &CandidateSnapshot,
    artifacts: &[(&str, Option<&[u8]>)],
) -> Result<[u8; 32], &'static str> {
    let digest = candidate_revision_digest(snapshot)?;
    aexeo_contracts::content_gate_binding_v2::candidate_snapshot_binding(
        artifacts,
        Some(aexeo_contracts::content_gate_binding_v2::CANDIDATE_REVISION_PROFILE),
        Some(&digest),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn fixture() -> (CandidateSnapshot, String) {
        let value: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/candidate-revision-vector.json"
        ))
        .unwrap();
        (
            serde_json::from_value(value["snapshot"].clone()).unwrap(),
            value["sha256"].as_str().unwrap().into(),
        )
    }
    #[test]
    fn binding_matches_vector_and_changes_with_snapshot_and_decision_inputs() {
        let (snapshot, _) = fixture();
        let vector: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/candidate-binding-vector.json"
        ))
        .unwrap();
        let artifacts: [(&str, Option<&[u8]>); 6] = [
            ("change", Some("article: café".as_bytes())),
            ("resolved_policy", Some(b"policy-v1")),
            ("host_options", Some(b"{}")),
            ("evaluator", Some(b"engine-id")),
            ("collection", None),
            ("external_evidence", Some(b"")),
        ];
        let first = candidate_decision_binding(&snapshot, &artifacts).unwrap();
        assert_eq!(
            format!("{:x}", sha2::digest::Output::<sha2::Sha256>::from(first)),
            vector["sha256"].as_str().unwrap()
        );
        let mut scope = snapshot.clone();
        scope.scope.push('x');
        assert_ne!(
            first,
            candidate_decision_binding(&scope, &artifacts).unwrap()
        );
        let mut item = snapshot.clone();
        item.items[0].source.push('x');
        assert_ne!(
            first,
            candidate_decision_binding(&item, &artifacts).unwrap()
        );
        let mut removed = snapshot.clone();
        removed.items.pop();
        assert_ne!(
            first,
            candidate_decision_binding(&removed, &artifacts).unwrap()
        );
        let mut reordered = snapshot.clone();
        reordered.items.reverse();
        assert_eq!(
            first,
            candidate_decision_binding(&reordered, &artifacts).unwrap()
        );
        for index in 0..artifacts.len() {
            let mut changed = artifacts;
            changed[index].1 = Some(b"changed");
            assert_ne!(
                first,
                candidate_decision_binding(&snapshot, &changed).unwrap()
            );
        }
    }

    #[test]
    fn overlay_candidate_can_be_bound_and_recomputed_independently() {
        let (mut base, _) = fixture();
        let item = base.items[0].clone();
        base.items.clear();
        let change: aexeo_contracts::content_gate_protocol::Change = serde_json::from_value(serde_json::json!({
            "schema":"content-gate.change/v1", "change_id":"new",
            "created_at":"2026-01-01T00:00:00Z", "origin":{"kind":"manual"},
            "base":base.base, "site":{"id":base.site_id},
            "entries":[{"entity_id":item.entity_id, "operation":"create", "media_type":item.media_type,
                "public_path":item.public_path, "after":{"sha256":item.sha256, "inline":item.source}}]
        })).unwrap();
        let overlay = crate::content_candidate::overlay_candidate(
            &base,
            &change,
            std::slice::from_ref(&item),
        )
        .unwrap();
        let artifacts: [(&str, Option<&[u8]>); 6] = [
            ("change", Some(b"change")),
            ("resolved_policy", Some(b"policy")),
            ("host_options", Some(b"options")),
            ("evaluator", Some(b"build")),
            ("collection", None),
            ("external_evidence", None),
        ];
        let actual = candidate_decision_binding(&overlay.candidate, &artifacts).unwrap();
        let mut independently_built = base.clone();
        independently_built.items.push(item);
        assert_eq!(
            actual,
            candidate_decision_binding(&independently_built, &artifacts).unwrap()
        );
        independently_built.complete = !independently_built.complete;
        assert_ne!(
            actual,
            candidate_decision_binding(&independently_built, &artifacts).unwrap()
        );
    }

    #[test]
    fn candidate_profile_matches_independent_vector_and_binds_every_field() {
        let (snapshot, expected) = fixture();
        let digest = candidate_revision_digest(&snapshot).unwrap();
        assert_eq!(
            format!("{:x}", sha2::digest::Output::<sha2::Sha256>::from(digest)),
            expected
        );
        let mut reordered = snapshot.clone();
        reordered.items.reverse();
        assert_eq!(digest, candidate_revision_digest(&reordered).unwrap());
        let mut variants = Vec::new();
        let mut v = snapshot.clone();
        v.scope.push('x');
        variants.push(v);
        let mut v = snapshot.clone();
        v.site_id.push('x');
        variants.push(v);
        let mut v = snapshot.clone();
        v.site_base_url = None;
        variants.push(v);
        let mut v = snapshot.clone();
        v.base.kind = BaseKind::Git;
        variants.push(v);
        let mut v = snapshot.clone();
        v.base.revision.push('x');
        variants.push(v);
        let mut v = snapshot.clone();
        v.complete = false;
        variants.push(v);
        let mut v = snapshot.clone();
        v.items[0].entity_id.push('x');
        variants.push(v);
        let mut v = snapshot.clone();
        v.items[0].public_path.push('x');
        variants.push(v);
        let mut v = snapshot.clone();
        v.items[0].media_type.push('x');
        variants.push(v);
        let mut v = snapshot.clone();
        v.items[0].revision = Some(String::new());
        variants.push(v);
        let mut v = snapshot.clone();
        v.items[0].sha256.push('x');
        variants.push(v);
        let mut v = snapshot.clone();
        v.items[0].source.push('x');
        variants.push(v);
        let mut v = snapshot.clone();
        v.items[0].links.as_mut().unwrap().reverse();
        variants.push(v);
        let mut v = snapshot.clone();
        v.items[1].links = Some(vec![]);
        variants.push(v);
        let mut v = snapshot.clone();
        v.items.pop();
        variants.push(v);
        for variant in variants {
            assert_ne!(digest, candidate_revision_digest(&variant).unwrap());
        }
        let mut duplicate = snapshot.clone();
        duplicate.items.push(duplicate.items[0].clone());
        assert!(candidate_revision_digest(&duplicate).is_err());
    }
}
