//! Experimental exact-byte identity for a caller-supplied site revision set.
//!
//! This primitive cannot establish the completeness, provenance or freshness of
//! the supplied records. It is not itself a publication authorization.

use sha2::{Digest, Sha256};

const DOMAIN: &[u8] = b"content-gate:revision-set:v1\0";
const MAX_RECORDS: usize = 100_000;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ID_BYTES: usize = 4096;

/// Exact, independently obtained bytes representing one stable entity.
#[derive(Clone, Copy)]
pub struct RevisionRecord<'a> {
    pub entity_id: &'a str,
    pub record: &'a [u8],
}

/// Hash the exact scope, snapshot descriptor and entity records.
///
/// Entities are sorted by their UTF-8 ID bytes; duplicate IDs are rejected.
/// Neither the identity nor the presence of a record proves completeness.
/// Callers must agree on a record profile that covers every relevant field and
/// separately establish source authority, freshness and the publication boundary.
/// Scope and snapshot descriptors must also have agreed, reproducible bytes.
pub fn revision_set_digest(
    scope: &[u8],
    snapshot: &[u8],
    entries: &[RevisionRecord<'_>],
) -> Result<[u8; 32], &'static str> {
    if scope.is_empty() || snapshot.is_empty() {
        return Err("scope and snapshot descriptors must not be empty");
    }
    if entries.len() > MAX_RECORDS {
        return Err("too many revision records");
    }
    let mut total = scope
        .len()
        .checked_add(snapshot.len())
        .ok_or("revision set exceeds size limit")?;
    let mut sorted = entries.to_vec();
    sorted.sort_unstable_by(|a, b| a.entity_id.as_bytes().cmp(b.entity_id.as_bytes()));
    let mut previous: Option<&str> = None;
    for entry in &sorted {
        if entry.entity_id.is_empty()
            || entry.entity_id.len() > MAX_ID_BYTES
            || entry.entity_id.chars().any(char::is_control)
        {
            return Err("invalid entity ID");
        }
        if previous == Some(entry.entity_id) {
            return Err("duplicate entity ID");
        }
        previous = Some(entry.entity_id);
        total = total
            .checked_add(entry.entity_id.len())
            .and_then(|n| n.checked_add(entry.record.len()))
            .ok_or("revision set exceeds size limit")?;
    }
    if total > MAX_BYTES {
        return Err("revision set exceeds size limit");
    }
    let mut hash = Sha256::new();
    hash.update(DOMAIN);
    hash.update((scope.len() as u64).to_be_bytes());
    hash.update(scope);
    hash.update((snapshot.len() as u64).to_be_bytes());
    hash.update(snapshot);
    hash.update((sorted.len() as u32).to_be_bytes());
    for entry in sorted {
        hash.update((entry.entity_id.len() as u32).to_be_bytes());
        hash.update(entry.entity_id.as_bytes());
        hash.update((entry.record.len() as u64).to_be_bytes());
        hash.update(entry.record);
    }
    Ok(hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_vector_and_mutations() {
        let scope = b"site=aeptus;surface=preview";
        let snapshot = b"revision=2026-11-18";
        let a = RevisionRecord {
            entity_id: "post:caf\u{e9}",
            record: b"title=caf\xc3\xa9\npath=/fr",
        };
        let b = RevisionRecord {
            entity_id: "page:home",
            record: b"title=Home\npath=/",
        };
        let original = revision_set_digest(scope, snapshot, &[a, b]).unwrap();
        let vector: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/revision-set-vector.json"))
                .unwrap();
        assert_eq!(
            format!("{:x}", sha2::digest::Output::<Sha256>::from(original)),
            vector["sha256"].as_str().unwrap()
        );
        assert_eq!(
            original,
            revision_set_digest(scope, snapshot, &[b, a]).unwrap()
        );
        assert_ne!(
            original,
            revision_set_digest(b"another scope", snapshot, &[a, b]).unwrap()
        );
        assert_ne!(
            original,
            revision_set_digest(scope, b"another snapshot", &[a, b]).unwrap()
        );
        assert_ne!(
            original,
            revision_set_digest(scope, snapshot, &[a]).unwrap()
        );
        assert_ne!(
            original,
            revision_set_digest(
                scope,
                snapshot,
                &[
                    RevisionRecord {
                        record: b"changed",
                        ..a
                    },
                    b
                ]
            )
            .unwrap()
        );
        assert_ne!(
            original,
            revision_set_digest(
                scope,
                snapshot,
                &[
                    RevisionRecord {
                        entity_id: "new",
                        ..a
                    },
                    b
                ]
            )
            .unwrap()
        );
        assert!(revision_set_digest(scope, snapshot, &[a, a]).is_err());
        assert!(revision_set_digest(b"", snapshot, &[a]).is_err());
        assert!(revision_set_digest(scope, b"", &[a]).is_err());
        assert!(
            revision_set_digest(
                scope,
                snapshot,
                &[RevisionRecord {
                    entity_id: "\n",
                    ..a
                }]
            )
            .is_err()
        );
        assert!(
            revision_set_digest(scope, snapshot, &[RevisionRecord { entity_id: "", ..a }]).is_err()
        );
        let oversized = vec![0; MAX_BYTES + 1];
        assert!(
            revision_set_digest(
                scope,
                snapshot,
                &[RevisionRecord {
                    record: &oversized,
                    ..a
                }]
            )
            .is_err()
        );
        assert!(revision_set_digest(scope, snapshot, &vec![a; MAX_RECORDS + 1]).is_err());
    }
}
