//! Experimental exact-byte decision-input binding. Not a verdict verifier.

use sha2::{Digest, Sha256};

const DOMAIN: &[u8] = b"content-gate:decision-inputs:v2\0";
const MAX_ARTIFACTS: usize = 256;
const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
const REQUIRED: [&str; 6] = [
    "change",
    "resolved_policy",
    "host_options",
    "evaluator",
    "collection",
    "external_evidence",
];

/// Bind explicitly named artifact bytes in deterministic name order.
///
/// This experimental primitive does not verify a verdict, establish which
/// dependencies were required, authenticate bytes, or determine freshness.
/// The caller must independently obtain the complete current artifact set.
pub fn decision_binding(artifacts: &[(&str, Option<&[u8]>)]) -> Result<[u8; 32], &'static str> {
    if artifacts.len() > MAX_ARTIFACTS {
        return Err("too many decision artifacts");
    }
    let mut sorted = artifacts.to_vec();
    sorted.sort_unstable_by_key(|(name, _)| *name);
    let mut total = 0usize;
    let mut previous = None;
    for (name, bytes) in &sorted {
        if name.is_empty()
            || name.len() > 64
            || !name.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
            })
        {
            return Err("invalid decision artifact name");
        }
        if previous == Some(*name) {
            return Err("duplicate decision artifact name");
        }
        previous = Some(*name);
        if let Some(bytes) = bytes {
            total = total
                .checked_add(bytes.len())
                .ok_or("decision artifacts exceed size limit")?;
            if total > MAX_ARTIFACT_BYTES {
                return Err("decision artifacts exceed size limit");
            }
        }
    }
    for name in REQUIRED {
        match sorted.iter().find(|(role, _)| *role == name) {
            None => return Err("required decision artifact is missing"),
            Some((_, None)) if !matches!(name, "collection" | "external_evidence") => {
                return Err("required decision artifact is absent");
            }
            _ => {}
        }
    }
    let mut hash = Sha256::new();
    hash.update(DOMAIN);
    hash.update((sorted.len() as u32).to_be_bytes());
    for (name, bytes) in sorted {
        hash.update((name.len() as u32).to_be_bytes());
        hash.update(name.as_bytes());
        match bytes {
            None => hash.update([0]),
            Some(bytes) => {
                hash.update([1]);
                hash.update((bytes.len() as u64).to_be_bytes());
                hash.update(bytes);
            }
        }
    }
    Ok(hash.finalize().into())
}

/// Profile identifier for the documented CandidateSnapshot field encoding.
pub const CANDIDATE_REVISION_PROFILE: &str = "content-gate.candidate-revision/v1";

/// Bind an independently computed candidate revision digest to the explicit
/// decision artifact bytes. This is an experimental composition, not a v2
/// verdict or publication authorization. The digest must be independently
/// computed from the current snapshot; a digest copied from a prior verdict
/// does not establish freshness, authenticity or completeness.
///
/// Both profile and digest must be present. The two artifact names are reserved
/// and must not appear in `artifacts`. Unknown profiles and invalid digest
/// lengths fail closed, as do all violations of `decision_binding`.
pub fn candidate_snapshot_binding(
    artifacts: &[(&str, Option<&[u8]>)],
    profile: Option<&str>,
    digest: Option<&[u8]>,
) -> Result<[u8; 32], &'static str> {
    if profile != Some(CANDIDATE_REVISION_PROFILE) {
        return Err("missing or unsupported candidate revision profile");
    }
    let digest = digest.ok_or("missing candidate revision digest")?;
    if digest.len() != 32 {
        return Err("candidate revision digest must be 32 bytes");
    }
    if artifacts.iter().any(|(name, _)| {
        matches!(
            *name,
            "candidate_revision_profile" | "candidate_revision_sha256"
        )
    }) {
        return Err("candidate revision artifact names are reserved");
    }
    let mut bound = artifacts.to_vec();
    bound.push((
        "candidate_revision_profile",
        Some(CANDIDATE_REVISION_PROFILE.as_bytes()),
    ));
    bound.push(("candidate_revision_sha256", Some(digest)));
    decision_binding(&bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_binding_refuses_missing_unknown_and_shadowed_inputs() {
        let artifacts: [(&str, Option<&[u8]>); 6] = [
            ("change", Some(b"change")),
            ("resolved_policy", Some(b"policy")),
            ("host_options", Some(b"options")),
            ("evaluator", Some(b"build")),
            ("collection", None),
            ("external_evidence", None),
        ];
        let digest = [1u8; 32];
        let valid =
            candidate_snapshot_binding(&artifacts, Some(CANDIDATE_REVISION_PROFILE), Some(&digest))
                .unwrap();
        assert!(candidate_snapshot_binding(&artifacts, None, Some(&digest)).is_err());
        assert!(
            candidate_snapshot_binding(&artifacts, Some("unsupported/v2"), Some(&digest)).is_err()
        );
        assert!(
            candidate_snapshot_binding(&artifacts, Some(CANDIDATE_REVISION_PROFILE), None).is_err()
        );
        assert!(
            candidate_snapshot_binding(
                &artifacts,
                Some(CANDIDATE_REVISION_PROFILE),
                Some(&digest[..31])
            )
            .is_err()
        );
        let mut shadow = artifacts.to_vec();
        shadow.push(("candidate_revision_profile", Some(b"other")));
        assert!(
            candidate_snapshot_binding(&shadow, Some(CANDIDATE_REVISION_PROFILE), Some(&digest))
                .is_err()
        );
        shadow.pop();
        shadow.push(("candidate_revision_sha256", Some(&digest)));
        assert!(
            candidate_snapshot_binding(&shadow, Some(CANDIDATE_REVISION_PROFILE), Some(&digest))
                .is_err()
        );
        let changed = [2u8; 32];
        assert_ne!(
            valid,
            candidate_snapshot_binding(
                &artifacts,
                Some(CANDIDATE_REVISION_PROFILE),
                Some(&changed)
            )
            .unwrap()
        );
        for index in 0..artifacts.len() {
            let mut modified = artifacts;
            modified[index].1 = Some(b"changed");
            assert_ne!(
                valid,
                candidate_snapshot_binding(
                    &modified,
                    Some(CANDIDATE_REVISION_PROFILE),
                    Some(&digest)
                )
                .unwrap()
            );
        }
    }

    #[test]
    fn bindings_are_stable_and_track_every_artifact() {
        let artifacts = [
            ("change", Some(&b"article: caf\xc3\xa9"[..])),
            ("resolved_policy", Some(&b"policy-v1"[..])),
            ("host_options", Some(&b"{}"[..])),
            ("evaluator", Some(&b"engine-id"[..])),
            ("collection", None),
            ("external_evidence", Some(&b""[..])),
        ];
        let first = decision_binding(&artifacts).unwrap();
        let vector: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/decision-binding-vector.json"
        ))
        .unwrap();
        assert_eq!(
            format!("{:x}", sha2::digest::Output::<Sha256>::from(first)),
            vector["sha256"].as_str().unwrap()
        );
        for index in 0..artifacts.len() {
            let mut changed = artifacts;
            changed[index].1 = Some(b"other");
            assert_ne!(first, decision_binding(&changed).unwrap());
        }
        let mut extra = artifacts.to_vec();
        extra.push(("custom.dep", Some(b"evidence")));
        assert_ne!(first, decision_binding(&extra).unwrap());
        let mut reversed = artifacts;
        reversed.reverse();
        assert_eq!(first, decision_binding(&reversed).unwrap());
        reversed[0].1 = Some(b"changed");
        assert_ne!(first, decision_binding(&reversed).unwrap());
        let mut absent = artifacts;
        absent[5].1 = None;
        assert_ne!(first, decision_binding(&absent).unwrap());
        assert!(decision_binding(&artifacts[..5]).is_err());
        let mut duplicate = artifacts.to_vec();
        duplicate.push(artifacts[0]);
        assert!(decision_binding(&duplicate).is_err());
        let mut invalid = artifacts;
        invalid[0].0 = "Bad Name";
        assert!(decision_binding(&invalid).is_err());
        let mut missing = artifacts;
        missing[0].1 = None;
        assert!(decision_binding(&missing).is_err());
        let too_many = vec![("x", None); MAX_ARTIFACTS + 1];
        assert!(decision_binding(&too_many).is_err());
        let oversized = vec![0; MAX_ARTIFACT_BYTES + 1];
        let mut large = artifacts;
        large[0].1 = Some(&oversized);
        assert!(decision_binding(&large).is_err());
        let long_name = "x".repeat(65);
        let mut invalid = artifacts;
        invalid[0].0 = &long_name;
        assert!(decision_binding(&invalid).is_err());
    }
}
