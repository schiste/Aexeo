//! Draft portable bound-verdict framing; not a publication verifier.
//!
//! A matching binding does not authenticate inputs or establish freshness,
//! inventory completeness, dependency closure or publication atomicity.

use crate::content_gate_binding_v2::{CANDIDATE_REVISION_PROFILE, candidate_snapshot_binding};
use crate::content_gate_change_validation::{parse_lossless_json, parse_strict_validated_change};
use crate::content_gate_decision_inputs::DecisionInputBytes;
use crate::content_gate_protocol::{CheckStatus, Subject, VERDICT_SCHEMA, Verdict, VerdictStatus};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "content-gate.bound-verdict/v2";
pub const PROFILE: &str = "content-gate.candidate-binding/v1";
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// This draft profile permits the informational Aexeo offline-binding and
/// explicit HTML main-scope extensions; other extensions are rejected.
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

/// Independently supplied expectations for an experimental, passing result.
/// Labels and exit status are caller assertions, not authenticated identity or execution proof.
pub struct PassingRequirements<'a> {
    pub validator_name: &'a str,
    pub validator_version: &'a str,
    pub required_checks: &'a [&'a str],
    pub exit_code: i32,
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

fn is_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl BoundVerdict {
    /// Construct framing for an already evaluated, structurally consistent verdict.
    /// This does not validate the origin or completeness of the inputs.
    pub fn new(verdict: Verdict, inputs: &CurrentInputs<'_>) -> Result<Self, &'static str> {
        let result = Self {
            schema: SCHEMA.into(),
            binding_profile: PROFILE.into(),
            binding_sha256: binding(inputs)?,
            verdict,
        };
        result.validate_verdict()?;
        Ok(result)
    }

    /// Reject oversized, ambiguous, unsupported or inconsistent draft envelopes.
    /// This does not authenticate inputs or authorize publication.
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > MAX_BYTES {
            return Err("bound verdict exceeds size limit");
        }
        let envelope: Self = parse_lossless_json(bytes).map_err(|_| "invalid bound verdict")?;
        envelope.validate_verdict()?;
        Ok(envelope)
    }

    fn validate_header(&self) -> Result<(), &'static str> {
        if self.schema != SCHEMA || self.binding_profile != PROFILE {
            return Err("unsupported bound verdict schema or profile");
        }
        if !is_sha256(&self.binding_sha256) {
            return Err("invalid binding digest");
        }
        if self.verdict.schema != VERDICT_SCHEMA {
            return Err("unsupported embedded verdict schema");
        }
        if let Some(extensions) = &self.verdict.extensions {
            let digest = extensions
                .get("aexeo.offline-input-binding.v1")
                .and_then(serde_json::Value::as_str);
            if !digest.is_some_and(is_sha256)
                || extensions.keys().any(|key| {
                    key != "aexeo.offline-input-binding.v1" && key != "aexeo.html-content-scope.v1"
                })
                || extensions
                    .get("aexeo.html-content-scope.v1")
                    .is_some_and(|value| value.as_str() != Some("main"))
            {
                return Err("unsupported or malformed verdict extension");
            }
        }
        Ok(())
    }

    /// Validate the embedded verdict's supported fields and internal invariants.
    /// This does not verify its subject against a change, required checks, trusted
    /// validator identity, evaluator exit code, or publication authorization.
    pub fn validate_verdict(&self) -> Result<(), &'static str> {
        self.validate_header()?;
        let v = &self.verdict;
        if v.schema != VERDICT_SCHEMA
            || v.subject.change_id.is_empty()
            || v.subject.base.revision.is_empty()
            || v.subject.entries.is_empty()
            || v.validator.name.is_empty()
            || v.validator.version.is_empty()
            || v.validator.profile.as_ref().is_some_and(String::is_empty)
            || v.validator
                .profile_sha256
                .as_ref()
                .is_some_and(|s| !is_sha256(s))
            || v.reason.as_ref().is_some_and(String::is_empty)
            || (matches!(v.status, VerdictStatus::NotChecked | VerdictStatus::Error)
                && v.reason.is_none())
            || time::OffsetDateTime::parse(
                &v.evaluated_at,
                &time::format_description::well_known::Rfc3339,
            )
            .is_err()
        {
            return Err("invalid embedded verdict fields");
        }
        let mut entities = BTreeSet::new();
        for entry in &v.subject.entries {
            if entry.entity_id.is_empty()
                || !entities.insert(entry.entity_id.as_str())
                || entry.sha256.as_ref().is_some_and(|s| !is_sha256(s))
            {
                return Err("invalid or duplicate verdict subject entry");
            }
        }
        let mut ids = BTreeSet::new();
        let (
            mut passed,
            mut failed,
            mut not_checked,
            mut blocking_failed,
            mut blocking_not_checked,
        ) = (0u64, 0u64, 0u64, 0u64, 0u64);
        for check in &v.checks {
            if check.id.is_empty() || !ids.insert(check.id.as_str()) {
                return Err("invalid or duplicate check ID");
            }
            match check.status {
                CheckStatus::Passed => passed += 1,
                CheckStatus::Failed => {
                    failed += 1;
                    if check.blocking {
                        blocking_failed += 1;
                    }
                }
                CheckStatus::NotChecked => {
                    not_checked += 1;
                    if check.blocking {
                        blocking_not_checked += 1;
                    }
                }
            }
        }
        for finding in &v.findings {
            if finding.rule_id.is_empty()
                || finding.message.is_empty()
                || finding.entity_id.as_ref().is_some_and(String::is_empty)
                || finding
                    .check_id
                    .as_ref()
                    .is_some_and(|id| id.is_empty() || !ids.contains(id.as_str()))
                || finding
                    .location
                    .as_ref()
                    .is_some_and(|loc| loc.line == 0 || loc.column == 0)
            {
                return Err("invalid verdict finding");
            }
        }
        let summary = &v.summary;
        if summary.checks != v.checks.len() as u64
            || summary.passed != passed
            || summary.failed != failed
            || summary.not_checked != not_checked
            || summary.blocking_failed != blocking_failed
            || summary.blocking_not_checked != blocking_not_checked
        {
            return Err("inconsistent verdict summary");
        }
        let expected = if blocking_not_checked > 0 {
            VerdictStatus::NotChecked
        } else if blocking_failed > 0 {
            VerdictStatus::Fail
        } else {
            VerdictStatus::Pass
        };
        if v.status != VerdictStatus::Error && v.status != expected {
            return Err("inconsistent verdict status");
        }
        Ok(())
    }

    /// Check independently supplied inputs, the declared subject and explicit
    /// passing requirements. This is not publication authorization: the caller
    /// must establish trusted execution, content bytes, freshness, completeness,
    /// dependency closure and atomicity with publication independently.
    pub fn verify_passing(
        &self,
        inputs: &CurrentInputs<'_>,
        required: &PassingRequirements<'_>,
    ) -> Result<(), &'static str> {
        self.verify_inputs(inputs)?;
        if required.validator_name.is_empty()
            || required.validator_version.is_empty()
            || required.required_checks.is_empty()
            || required.exit_code != 0
        {
            return Err("invalid or unsuccessful verification requirements");
        }
        let change = parse_strict_validated_change(inputs.artifacts.change)
            .map_err(|_| "invalid current change")?;
        if change.extensions.is_some() {
            return Err("unsupported change extensions");
        }
        if self.verdict.subject != Subject::for_change(&change) {
            return Err("verdict subject does not match current change");
        }
        if self.verdict.validator.name != required.validator_name
            || self.verdict.validator.version != required.validator_version
        {
            return Err("validator labels do not match requirements");
        }
        if self.verdict.status != VerdictStatus::Pass {
            return Err("verdict is not passing");
        }
        let checks: BTreeMap<_, _> = self
            .verdict
            .checks
            .iter()
            .map(|check| (check.id.as_str(), check))
            .collect();
        let mut ids = BTreeSet::new();
        for id in required.required_checks {
            if id.is_empty() || !ids.insert(*id) {
                return Err("invalid required check IDs");
            }
            if !checks
                .get(id)
                .is_some_and(|check| check.blocking && check.status == CheckStatus::Passed)
            {
                return Err("required blocking check did not pass");
            }
        }
        Ok(())
    }

    /// Compare a binding recomputed solely from current caller-supplied inputs.
    /// A match does not establish a passing verdict or authorize publication.
    pub fn verify_inputs(&self, inputs: &CurrentInputs<'_>) -> Result<(), &'static str> {
        self.validate_verdict()?;
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
        serde_json::from_value(vector["candidate"]["receipt"]["verdict"].clone()).unwrap()
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
    fn rejects_inconsistent_embedded_verdicts_before_reuse() {
        let envelope = BoundVerdict::new(verdict(), &inputs()).unwrap();
        let original = serde_json::to_value(&envelope).unwrap();
        type VerdictMutation = Box<dyn Fn(&mut serde_json::Value)>;
        let variants: Vec<VerdictMutation> = vec![
            Box::new(|v| v["verdict"]["summary"]["passed"] = 999.into()),
            Box::new(|v| v["verdict"]["status"] = "fail".into()),
            Box::new(|v| v["verdict"]["evaluated_at"] = "not-a-date".into()),
            Box::new(|v| v["verdict"]["validator"]["name"] = "".into()),
            Box::new(|v| v["verdict"]["subject"]["entries"][0]["sha256"] = "bad".into()),
            Box::new(|v| {
                let check = v["verdict"]["checks"][0].clone();
                v["verdict"]["checks"].as_array_mut().unwrap().push(check);
            }),
            Box::new(
                |v| v["verdict"]["findings"] = serde_json::json!([{ "rule_id": "r", "severity": "warning", "message": "m", "check_id": "missing" }]),
            ),
            Box::new(
                |v| v["verdict"]["findings"] = serde_json::json!([{ "rule_id": "r", "severity": "warning", "message": "m", "location": {"line": 0, "column": 1} }]),
            ),
            Box::new(|v| {
                let entry = v["verdict"]["subject"]["entries"][0].clone();
                v["verdict"]["subject"]["entries"]
                    .as_array_mut()
                    .unwrap()
                    .push(entry);
            }),
            Box::new(|v| v["verdict"]["status"] = "error".into()),
        ];
        for mutate in variants {
            let mut changed = original.clone();
            mutate(&mut changed);
            assert!(
                BoundVerdict::parse(&serde_json::to_vec(&changed).unwrap()).is_err(),
                "{changed}"
            );
        }
        let mut modified = envelope;
        modified.verdict.summary.passed += 1;
        assert!(modified.verify_inputs(&inputs()).is_err());
        assert!(BoundVerdict::new(modified.verdict, &inputs()).is_err());
    }

    #[test]
    fn accepts_consistent_nonpassing_verdicts_without_approving_them() {
        let mut v = verdict();
        v.checks[0].status = CheckStatus::NotChecked;
        v.summary.passed = 0;
        v.summary.not_checked = 1;
        v.summary.blocking_not_checked = 1;
        v.status = VerdictStatus::NotChecked;
        v.reason = Some("not observed".into());
        let bound = BoundVerdict::new(v, &inputs()).unwrap();
        let parsed = BoundVerdict::parse(&serde_json::to_vec(&bound).unwrap()).unwrap();
        assert_eq!(parsed.verdict.status, VerdictStatus::NotChecked);
        assert!(parsed.verify_inputs(&inputs()).is_ok());
    }

    #[test]
    fn passing_verification_checks_the_bound_change_and_host_requirements() {
        let vector: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/specs/content-gate/v2/candidate-verdict-vector.json"
        ))
        .unwrap();
        let change = vector["artifacts"]["change"].as_str().unwrap().as_bytes();
        let input = CurrentInputs {
            artifacts: DecisionInputBytes {
                change,
                resolved_policy: b"policy",
                host_options: b"options",
                evaluator: b"build",
                collection: None,
                external_evidence: None,
            },
            base_revision_sha256: &[1; 32],
            candidate_revision_sha256: &[2; 32],
        };
        let ids = ["gate:7:article:words"];
        let requirements = PassingRequirements {
            validator_name: "aexeo",
            validator_version: "0.0.24",
            required_checks: &ids,
            exit_code: 0,
        };
        let original = verdict();
        let envelope = BoundVerdict::new(original.clone(), &input).unwrap();
        assert!(envelope.verify_passing(&input, &requirements).is_ok());
        for mutate in [0, 1, 2, 3] {
            let mut v = original.clone();
            match mutate {
                0 => v.subject.change_id = "other".into(),
                1 => v.subject.base.revision = "other".into(),
                2 => v.subject.entries[0].entity_id = "other".into(),
                _ => v.subject.entries[0].sha256 = None,
            }
            let altered = BoundVerdict::new(v, &input).unwrap();
            assert!(altered.verify_inputs(&input).is_ok());
            assert!(altered.verify_passing(&input, &requirements).is_err());
        }
        for (name, version, checks, exit) in [
            ("other", "0.0.24", &ids[..], 0),
            ("aexeo", "other", &ids[..], 0),
            ("aexeo", "0.0.24", &ids[..], 1),
            ("aexeo", "0.0.24", &[][..], 0),
            ("aexeo", "0.0.24", &["missing"][..], 0),
            ("aexeo", "0.0.24", &["", ids[0]][..], 0),
            ("aexeo", "0.0.24", &[ids[0], ids[0]][..], 0),
        ] {
            let req = PassingRequirements {
                validator_name: name,
                validator_version: version,
                required_checks: checks,
                exit_code: exit,
            };
            assert!(envelope.verify_passing(&input, &req).is_err());
        }
        let mut advisory = original.clone();
        advisory.checks[0].blocking = false;
        assert!(
            BoundVerdict::new(advisory, &input)
                .unwrap()
                .verify_passing(&input, &requirements)
                .is_err()
        );
        let mut unchecked = original;
        unchecked.checks[0].status = CheckStatus::NotChecked;
        unchecked.summary.passed = 0;
        unchecked.summary.not_checked = 1;
        unchecked.summary.blocking_not_checked = 1;
        unchecked.status = VerdictStatus::NotChecked;
        unchecked.reason = Some("unavailable".into());
        assert!(
            BoundVerdict::new(unchecked, &input)
                .unwrap()
                .verify_passing(&input, &requirements)
                .is_err()
        );
        let mut with_extensions: serde_json::Value = serde_json::from_slice(change).unwrap();
        with_extensions["extensions"] = serde_json::json!({"future.input": true});
        let extended = serde_json::to_vec(&with_extensions).unwrap();
        let duplicate = String::from_utf8(change.to_vec()).unwrap().replacen(
            "{",
            "{\"change_id\":\"duplicate\",",
            1,
        );
        for bytes in [extended.as_slice(), duplicate.as_bytes()] {
            let invalid = CurrentInputs {
                artifacts: DecisionInputBytes {
                    change: bytes,
                    resolved_policy: b"policy",
                    host_options: b"options",
                    evaluator: b"build",
                    collection: None,
                    external_evidence: None,
                },
                base_revision_sha256: &[1; 32],
                candidate_revision_sha256: &[2; 32],
            };
            let bound = BoundVerdict::new(verdict(), &invalid).unwrap();
            assert!(bound.verify_inputs(&invalid).is_ok());
            assert!(bound.verify_passing(&invalid, &requirements).is_err());
        }
        let changed = CurrentInputs {
            artifacts: DecisionInputBytes {
                change: b"{}",
                ..input.artifacts
            },
            ..input
        };
        assert!(envelope.verify_passing(&changed, &requirements).is_err());
        // Even an envelope freshly bound to malformed change bytes cannot pass.
        let malformed = BoundVerdict::new(verdict(), &changed).unwrap();
        assert!(malformed.verify_passing(&changed, &requirements).is_err());
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
        let mut scoped: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        scoped["verdict"]["extensions"]["aexeo.html-content-scope.v1"] = "main".into();
        assert!(BoundVerdict::parse(&serde_json::to_vec(&scoped).unwrap()).is_ok());
        for invalid in [
            serde_json::json!("document"),
            serde_json::json!(null),
            serde_json::json!(true),
        ] {
            scoped["verdict"]["extensions"]["aexeo.html-content-scope.v1"] = invalid;
            assert!(BoundVerdict::parse(&serde_json::to_vec(&scoped).unwrap()).is_err());
        }
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
