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

#[cfg(test)]
mod tests {
    use super::*;

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
