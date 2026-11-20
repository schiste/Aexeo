//! Draft portable bound-verdict framing; not a publication verifier.
//!
//! This API is not connected to a CLI or runner. A matching binding does not
//! validate the embedded verdict, authenticate inputs or establish freshness,
//! inventory completeness, dependency closure or publication atomicity.

use crate::content_gate_binding_v2::{CANDIDATE_REVISION_PROFILE, candidate_snapshot_binding};
use crate::content_gate_change_validation::parse_lossless_json;
use crate::content_gate_decision_inputs::DecisionInputBytes;
use crate::content_gate_protocol::{VERDICT_SCHEMA, Verdict};
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "content-gate.bound-verdict/v2";
pub const PROFILE: &str = "content-gate.candidate-binding/v1";
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// This draft profile currently defines no verdict extensions. Unknown
/// extensions are rejected until their syntax and semantics are specified.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundVerdict {
    pub schema: String,
    pub binding_profile: String,
    pub binding_sha256: String,
    pub verdict: Verdict,
}

/// Independently acquired exact decision artifacts and independently computed
/// revision digests. Neither digest may be copied from the previous envelope.
pub struct CurrentInputs<'a> {
    pub artifacts: DecisionInputBytes<'a>,
    pub base_revision_sha256: &'a [u8; 32],
    pub candidate_revision_sha256: &'a [u8; 32],
}

fn binding(inputs: &CurrentInputs<'_>) -> Result<String, &'static str> {
    let a = &inputs.artifacts;
    if a.evaluator.is_empty() {
        return Err("empty evaluator identity");
    }
    let bytes = candidate_snapshot_binding(
        &[
            ("change", Some(a.change)),
            ("resolved_policy", Some(a.resolved_policy)),
            ("host_options", Some(a.host_options)),
            ("evaluator", Some(a.evaluator)),
            ("collection", a.collection),
            ("external_evidence", a.external_evidence),
            (
                "candidate_base_revision_sha256",
                Some(inputs.base_revision_sha256),
            ),
        ],
        Some(CANDIDATE_REVISION_PROFILE),
        Some(inputs.candidate_revision_sha256),
    )?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl BoundVerdict {
    /// Construct framing for an already evaluated verdict. This does not
    /// validate the verdict or the origin and completeness of the inputs.
    pub fn new(verdict: Verdict, inputs: &CurrentInputs<'_>) -> Result<Self, &'static str> {
        let result = Self {
            schema: SCHEMA.into(),
            binding_profile: PROFILE.into(),
            binding_sha256: binding(inputs)?,
            verdict,
        };
        result.validate_header()?;
        Ok(result)
    }

    /// Reject oversized, duplicate-key, unknown-field and unsupported draft
    /// envelopes. This does not validate the v1 verdict's schema or semantics.
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > MAX_BYTES {
            return Err("bound verdict exceeds size limit");
        }
        let envelope: Self = parse_lossless_json(bytes).map_err(|_| "invalid bound verdict")?;
        envelope.validate_header()?;
        Ok(envelope)
    }

    fn validate_header(&self) -> Result<(), &'static str> {
        if self.schema != SCHEMA || self.binding_profile != PROFILE {
            return Err("unsupported bound verdict schema or profile");
        }
        if self.binding_sha256.len() != 64
            || !self
                .binding_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid binding digest");
        }
        if self.verdict.schema != VERDICT_SCHEMA {
            return Err("unsupported embedded verdict schema");
        }
        if self.verdict.extensions.is_some() {
            return Err("verdict extensions are not supported by this profile");
        }
        Ok(())
    }

    /// Compare a binding recomputed solely from current caller-supplied inputs.
    /// A match is not a verdict validation or publication authorization.
    pub fn verify_inputs(&self, inputs: &CurrentInputs<'_>) -> Result<(), &'static str> {
        self.validate_header()?;
        if self.binding_sha256 != binding(inputs)? {
            return Err("decision inputs differ");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict() -> Verdict {
        let vector: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/specs/content-gate/v2/candidate-verdict-vector.json"
        ))
        .unwrap();
        let mut verdict = vector["candidate"]["receipt"]["verdict"].clone();
        verdict.as_object_mut().unwrap().remove("extensions");
        serde_json::from_value(verdict).unwrap()
    }

    fn inputs() -> CurrentInputs<'static> {
        CurrentInputs {
            artifacts: DecisionInputBytes {
                change: b"change",
                resolved_policy: b"policy",
                host_options: b"options",
                evaluator: b"build",
                collection: None,
                external_evidence: Some(b""),
            },
            base_revision_sha256: &[1; 32],
            candidate_revision_sha256: &[2; 32],
        }
    }

    #[test]
    fn roundtrip_requires_exact_current_inputs() {
        let envelope = BoundVerdict::new(verdict(), &inputs()).unwrap();
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let parsed = BoundVerdict::parse(&bytes).unwrap();
        assert_eq!(parsed, envelope);
        assert!(parsed.verify_inputs(&inputs()).is_ok());
        let mut changed = inputs();
        changed.artifacts.change = b"different";
        assert!(parsed.verify_inputs(&changed).is_err());
        for index in 0..4 {
            let mut changed = inputs();
            match index {
                0 => changed.artifacts.resolved_policy = b"different",
                1 => changed.artifacts.host_options = b"different",
                2 => changed.artifacts.evaluator = b"different",
                _ => changed.artifacts.external_evidence = None,
            }
            assert!(parsed.verify_inputs(&changed).is_err());
        }
        let mut changed = inputs();
        changed.artifacts.collection = Some(b"");
        assert!(parsed.verify_inputs(&changed).is_err());
        let mut changed = inputs();
        changed.base_revision_sha256 = &[3; 32];
        assert!(parsed.verify_inputs(&changed).is_err());
        let mut changed = inputs();
        changed.candidate_revision_sha256 = &[3; 32];
        assert!(parsed.verify_inputs(&changed).is_err());
        let mut bad_verdict = verdict();
        bad_verdict.schema = "unsupported".into();
        assert!(BoundVerdict::new(bad_verdict, &inputs()).is_err());
        let mut changed = inputs();
        changed.artifacts.evaluator = b"";
        assert!(BoundVerdict::new(verdict(), &changed).is_err());
    }

    #[test]
    fn matches_independent_candidate_binding_vector() {
        let vector: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/specs/content-gate/v2/candidate-verdict-vector.json"
        ))
        .unwrap();
        let a = &vector["artifacts"];
        let decode = |hex: &str| -> [u8; 32] {
            std::array::from_fn(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap())
        };
        let base = decode("69a706fb08665399c92496d0c4d7321e5d9857d469fc1ef0d8ee844526dd4ea7");
        let candidate = decode("5b3bbd7e90693296330978fdd708f309cc3c52bac6a8f17f450567d472b1c8a2");
        let input = CurrentInputs {
            artifacts: DecisionInputBytes {
                change: a["change"].as_str().unwrap().as_bytes(),
                resolved_policy: a["resolved_policy"].as_str().unwrap().as_bytes(),
                host_options: a["host_options"].as_str().unwrap().as_bytes(),
                evaluator: a["evaluator"].as_str().unwrap().as_bytes(),
                collection: None,
                external_evidence: None,
            },
            base_revision_sha256: &base,
            candidate_revision_sha256: &candidate,
        };
        assert_eq!(
            BoundVerdict::new(verdict(), &input).unwrap().binding_sha256,
            vector["candidate"]["binding"].as_str().unwrap()
        );
    }

    #[test]
    fn rejects_ambiguous_unknown_and_unsupported_envelopes() {
        let envelope = BoundVerdict::new(verdict(), &inputs()).unwrap();
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        let duplicate = text.replacen('{', "{\"schema\":\"content-gate.bound-verdict/v2\",", 1);
        assert!(BoundVerdict::parse(duplicate.as_bytes()).is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["verdict"]["subject"]["unknown"] = true.into();
        assert!(BoundVerdict::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["verdict"]["extensions"] = serde_json::json!({"unknown": true});
        assert!(BoundVerdict::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        for (field, invalid) in [
            ("schema", "content-gate.verdict/v1"),
            ("binding_profile", "unknown/v1"),
            ("binding_sha256", "A"),
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            value[field] = invalid.into();
            assert!(BoundVerdict::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        assert!(BoundVerdict::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
    }
}
