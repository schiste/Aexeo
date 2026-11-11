//! Explicitly supplied results from external content gate implementations.
//!
//! Results include the exact projected input and parameters they evaluated.
//! This detects stale inputs; it does not establish the producer's identity or
//! the correctness of its judgment. The caller must trust the producer.
use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::content_gate::{ContentGateExtensions, ContentInput, ExtensionDiagnostic};

const MAX_EXTERNAL_RESULTS: usize = 100;
const MAX_DIAGNOSTICS: usize = 1000;
const MAX_MESSAGE_BYTES: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalDiagnostic {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalGateResult {
    pub extension: String,
    pub parameters: BTreeMap<String, String>,
    pub input: ContentInput,
    pub diagnostics: Vec<ExternalDiagnostic>,
}

#[derive(Serialize)]
#[serde(transparent)]
pub struct ExternalGateResults(Vec<ExternalGateResult>);

impl ExternalGateResults {
    /// A fingerprint of the validated snapshot, including unconsumed records.
    /// It is not an authentication mechanism or evidence of producer quality.
    pub fn fingerprint(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        use std::io::{self, Write};
        struct HashWriter(Sha256);
        impl Write for HashWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.update(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut writer = HashWriter(Sha256::new());
        writer.0.update(b"aexeo.external-snapshot.v1");
        serde_json::to_writer(&mut writer, self)?;
        Ok(format!("{:x}", writer.0.finalize()))
    }

    pub fn new(results: Vec<ExternalGateResult>) -> Result<Self> {
        Self::validate(results, false)
    }

    /// Accept one result per entity, extension and parameter set for a batch.
    /// Each result still binds to the complete projected input.
    pub fn new_for_batch(results: Vec<ExternalGateResult>) -> Result<Self> {
        Self::validate(results, true)
    }

    fn validate(results: Vec<ExternalGateResult>, by_entity: bool) -> Result<Self> {
        if results.len() > MAX_EXTERNAL_RESULTS {
            bail!("too many external gate results");
        }
        for (index, result) in results.iter().enumerate() {
            if result.extension.trim().is_empty()
                || results[..index].iter().any(|other| {
                    other.extension == result.extension
                        && other.parameters == result.parameters
                        && (!by_entity || other.input.id == result.input.id)
                })
            {
                bail!("external gate identifiers and parameter sets must be nonempty and unique");
            }
            if result.diagnostics.len() > MAX_DIAGNOSTICS
                || result.diagnostics.iter().any(|d| {
                    d.message.trim().is_empty()
                        || d.message.len() > MAX_MESSAGE_BYTES
                        || d.line == 0
                        || d.column == 0
                })
            {
                bail!("external gate diagnostics are invalid or exceed limits");
            }
        }
        Ok(Self(results))
    }
}

impl ContentGateExtensions for ExternalGateResults {
    fn evaluate(
        &self,
        extension: &str,
        parameters: &BTreeMap<String, String>,
        input: &ContentInput,
    ) -> Result<Option<Vec<ExtensionDiagnostic>>> {
        let Some(result) = self.0.iter().find(|result| {
            result.extension == extension
                && &result.parameters == parameters
                && result.input.id == input.id
        }) else {
            return Ok(None);
        };
        if &result.input != input {
            return Ok(None);
        }
        if let Some(text) = &input.text {
            let mut required = BTreeMap::<usize, usize>::new();
            for diagnostic in &result.diagnostics {
                required
                    .entry(diagnostic.line)
                    .and_modify(|column| *column = (*column).max(diagnostic.column))
                    .or_insert(diagnostic.column);
            }
            for (index, line) in text
                .split('\n')
                .enumerate()
                .take_while(|_| !result.diagnostics.is_empty())
            {
                if let Some(column) = required.remove(&(index + 1)) {
                    if column > line.chars().count() + 1 {
                        bail!("external diagnostic column is outside projected text");
                    }
                    if required.is_empty() {
                        break;
                    }
                }
            }
            if !required.is_empty() {
                bail!("external diagnostic line is outside projected text");
            }
        } else if result
            .diagnostics
            .iter()
            .any(|d| d.line != 1 || d.column != 1)
        {
            bail!("external diagnostics without text must use position 1:1");
        }
        Ok(Some(
            result
                .diagnostics
                .iter()
                .map(|d| ExtensionDiagnostic {
                    message: d.message.clone(),
                    line: d.line,
                    column: d.column,
                })
                .collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> ExternalGateResult {
        ExternalGateResult {
            extension: "test.rule".into(),
            parameters: BTreeMap::from([("mode".into(), "strict".into())]),
            input: ContentInput {
                id: "draft".into(),
                text: Some("First line\nSecond".into()),
                ..Default::default()
            },
            diagnostics: vec![ExternalDiagnostic {
                message: "Review".into(),
                line: 2,
                column: 2,
            }],
        }
    }

    #[test]
    fn extension_binding_rejects_stale_input_and_parameter_changes() {
        let record = result();
        let host = ExternalGateResults::new(vec![record.clone()]).unwrap();
        assert_eq!(
            host.evaluate("test.rule", &record.parameters, &record.input)
                .unwrap()
                .unwrap()
                .len(),
            1
        );
        assert!(
            host.evaluate("test.rule", &BTreeMap::new(), &record.input)
                .unwrap()
                .is_none()
        );
        let mut changed = record.input.clone();
        changed.text = Some("Changed".into());
        assert!(
            host.evaluate("test.rule", &record.parameters, &changed)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn batch_results_are_unique_per_entity_and_still_bound_to_inputs() {
        let first = result();
        let mut second = first.clone();
        second.input.id = "other".into();
        second.input.text = Some("Other".into());
        second.diagnostics[0].line = 1;
        assert!(ExternalGateResults::new(vec![first.clone(), second.clone()]).is_err());
        let host = ExternalGateResults::new_for_batch(vec![first.clone(), second.clone()]).unwrap();
        assert!(
            host.evaluate("test.rule", &first.parameters, &first.input)
                .unwrap()
                .is_some()
        );
        assert!(
            host.evaluate("test.rule", &second.parameters, &second.input)
                .unwrap()
                .is_some()
        );
        second.input.text = Some("stale".into());
        assert!(
            host.evaluate("test.rule", &second.parameters, &second.input)
                .unwrap()
                .is_none()
        );
        let mut duplicate = first.clone();
        duplicate.input.text = Some("other snapshot".into());
        assert!(ExternalGateResults::new_for_batch(vec![first, duplicate]).is_err());
    }

    #[test]
    fn extension_rejects_duplicate_and_invalid_diagnostics() {
        let record = result();
        assert!(ExternalGateResults::new(vec![record.clone(), record.clone()]).is_err());
        let mut invalid = record.clone();
        invalid.diagnostics[0].column = 99;
        let host = ExternalGateResults::new(vec![invalid]).unwrap();
        assert!(
            host.evaluate("test.rule", &record.parameters, &record.input)
                .is_err()
        );
        let mut other = record.clone();
        other.parameters.insert("mode".into(), "different".into());
        assert!(ExternalGateResults::new(vec![record, other]).is_ok());
    }
}
