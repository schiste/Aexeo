//! Experimental portable digest manifest. This is not a verdict verifier.
//!
//! Callers independently acquire and protect every exact artifact, choose the
//! artifact formats, and validate freshness and completeness. Comparing hashes
//! alone cannot establish that an evaluator declared all its dependencies.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Experimental manifest identifier; not a content-gate verdict schema.
pub const DECISION_INPUTS_SCHEMA: &str = "content-gate.decision-inputs/v1";
const MAX_MANIFEST_BYTES: usize = 8192;

/// A component explicitly present or absent. Unknown observations are not absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Component {
    Absent {},
    Present { sha256: String },
}

impl Component {
    fn from_bytes(bytes: Option<&[u8]>) -> Self {
        match bytes {
            None => Self::Absent {},
            Some(bytes) => Self::Present {
                sha256: format!("{:x}", Sha256::digest(bytes)),
            },
        }
    }

    fn valid(&self) -> bool {
        match self {
            Self::Absent {} => true,
            Self::Present { sha256 } => {
                sha256.len() == 64
                    && sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }
        }
    }
}

/// Exact independently acquired artifact bytes; optional artifacts use `None`
/// only when the host has established they are not applicable.
pub struct DecisionInputBytes<'a> {
    pub change: &'a [u8],
    pub resolved_policy: &'a [u8],
    pub host_options: &'a [u8],
    pub evaluator: &'a [u8],
    pub collection: Option<&'a [u8]>,
    pub external_evidence: Option<&'a [u8]>,
}

/// Strict fixed-component manifest. The artifacts themselves are not embedded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionInputs {
    pub schema: String,
    pub change: Component,
    pub resolved_policy: Component,
    pub host_options: Component,
    pub evaluator: Component,
    pub collection: Component,
    pub external_evidence: Component,
}

impl DecisionInputs {
    pub fn from_bytes(inputs: &DecisionInputBytes<'_>) -> Self {
        Self {
            schema: DECISION_INPUTS_SCHEMA.into(),
            change: Component::from_bytes(Some(inputs.change)),
            resolved_policy: Component::from_bytes(Some(inputs.resolved_policy)),
            host_options: Component::from_bytes(Some(inputs.host_options)),
            evaluator: Component::from_bytes(Some(inputs.evaluator)),
            collection: Component::from_bytes(inputs.collection),
            external_evidence: Component::from_bytes(inputs.external_evidence),
        }
    }

    /// Parse a bounded manifest and reject malformed, missing and unknown fields.
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err("decision-input manifest exceeds size limit");
        }
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|_| "invalid decision-input manifest")?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != DECISION_INPUTS_SCHEMA {
            return Err("unsupported decision-input schema");
        }
        let required = [
            &self.change,
            &self.resolved_policy,
            &self.host_options,
            &self.evaluator,
        ];
        if required
            .iter()
            .any(|item| !matches!(item, Component::Present { .. }))
        {
            return Err("required decision input is absent");
        }
        if required
            .into_iter()
            .chain([&self.collection, &self.external_evidence])
            .any(|item| !item.valid())
        {
            return Err("invalid decision-input digest");
        }
        Ok(())
    }

    /// Compare against artifacts acquired independently by the caller.
    /// This does not validate the verdict, evidence freshness or artifact origin.
    pub fn verify(&self, inputs: &DecisionInputBytes<'_>) -> Result<(), &'static str> {
        self.validate()?;
        if *self != Self::from_bytes(inputs) {
            return Err("decision inputs differ");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> DecisionInputBytes<'static> {
        DecisionInputBytes {
            change: b"abc",
            resolved_policy: b"policy",
            host_options: b"options",
            evaluator: b"engine",
            collection: None,
            external_evidence: Some(b""),
        }
    }

    #[test]
    fn digest_matches_standard_vector_and_detects_changed_inputs() {
        let input = inputs();
        let manifest = DecisionInputs::from_bytes(&input);
        assert_eq!(
            manifest.change,
            Component::Present {
                sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()
            }
        );
        let json = serde_json::to_vec(&manifest).unwrap();
        let decoded = DecisionInputs::parse(&json).unwrap();
        assert!(decoded.verify(&input).is_ok());
        assert!(
            decoded
                .verify(&DecisionInputBytes {
                    change: b"abd",
                    ..inputs()
                })
                .is_err()
        );
        assert!(
            decoded
                .verify(&DecisionInputBytes {
                    resolved_policy: b"other",
                    ..inputs()
                })
                .is_err()
        );
        assert!(
            decoded
                .verify(&DecisionInputBytes {
                    host_options: b"other",
                    ..inputs()
                })
                .is_err()
        );
        assert!(
            decoded
                .verify(&DecisionInputBytes {
                    evaluator: b"other",
                    ..inputs()
                })
                .is_err()
        );
        assert!(
            decoded
                .verify(&DecisionInputBytes {
                    collection: Some(b""),
                    ..inputs()
                })
                .is_err()
        );
        assert!(
            decoded
                .verify(&DecisionInputBytes {
                    external_evidence: None,
                    ..inputs()
                })
                .is_err()
        );
    }

    #[test]
    fn python_example_matches_utf8_artifacts() {
        let example =
            include_bytes!("../../../docs/specs/content-gate/v2/decision-inputs-example.json");
        let parsed = DecisionInputs::parse(example).unwrap();
        let input = DecisionInputBytes {
            change: "article: café".as_bytes(),
            resolved_policy: b"policy-v1",
            host_options: b"{}",
            evaluator: b"engine-id",
            collection: None,
            external_evidence: Some(b""),
        };
        assert!(parsed.verify(&input).is_ok());
    }

    #[test]
    fn parsing_rejects_unknown_missing_duplicate_and_invalid_fields() {
        let manifest = DecisionInputs::from_bytes(&inputs());
        let mut value = serde_json::to_value(&manifest).unwrap();
        value["change"]["sha256"] = "A".repeat(64).into();
        assert!(DecisionInputs::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = serde_json::to_value(&manifest).unwrap();
        value["change"] = serde_json::json!({"state":"absent"});
        assert!(DecisionInputs::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = serde_json::to_value(&manifest).unwrap();
        value.as_object_mut().unwrap().remove("collection");
        assert!(DecisionInputs::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = serde_json::to_value(&manifest).unwrap();
        value["extra"] = true.into();
        assert!(DecisionInputs::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = serde_json::to_value(&manifest).unwrap();
        value["collection"] = serde_json::Value::Null;
        assert!(DecisionInputs::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = serde_json::to_value(&manifest).unwrap();
        value["collection"]["extra"] = true.into();
        assert!(DecisionInputs::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = serde_json::to_value(&manifest).unwrap();
        value["schema"] = "content-gate.decision-inputs/v2".into();
        assert!(DecisionInputs::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        let json = serde_json::to_string(&manifest).unwrap();
        let duplicate = json.replacen("{", "{\"schema\":\"content-gate.decision-inputs/v1\",", 1);
        assert!(DecisionInputs::parse(duplicate.as_bytes()).is_err());
        assert!(DecisionInputs::parse(&vec![b' '; MAX_MANIFEST_BYTES + 1]).is_err());
    }
}
