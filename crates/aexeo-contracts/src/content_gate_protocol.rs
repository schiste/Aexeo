//! Wire types for the draft content gate protocol v1.
//!
//! Deserialization checks field types and enumerations, but is not schema or
//! semantic validation. In particular, consumers must validate content sources,
//! paths, digests, subject binding, status consistency, and resource limits
//! before accepting or applying a verdict.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const CHANGE_SCHEMA: &str = "content-gate.change/v1";
pub const VERDICT_SCHEMA: &str = "content-gate.verdict/v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    #[serde(deserialize_with = "deserialize_change_schema")]
    pub schema: String,
    pub change_id: String,
    pub created_at: String,
    pub origin: Origin,
    pub base: Base,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<Site>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    pub entries: Vec<Entry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginKind {
    Git,
    Mcp,
    Api,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub kind: OriginKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseKind {
    Git,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    pub kind: BaseKind,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Site {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    Human,
    Agent,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<AuthorKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Create,
    Update,
    Delete,
    Rename,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub entity_id: String,
    pub operation: Operation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_entity_id: Option<String>,
    pub media_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<ContentState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<ContentState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    #[serde(deserialize_with = "deserialize_verdict_schema")]
    pub schema: String,
    pub status: VerdictStatus,
    pub subject: Subject,
    pub validator: Validator,
    pub evaluated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub summary: Summary,
    pub checks: Vec<Check>,
    pub findings: Vec<Finding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub findings_omitted: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictStatus {
    Pass,
    Fail,
    NotChecked,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject {
    pub change_id: String,
    pub base: Base,
    pub entries: Vec<SubjectEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectEntry {
    pub entity_id: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validator {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub checks: u64,
    pub passed: u64,
    pub failed: u64,
    pub not_checked: u64,
    pub blocking_failed: u64,
    pub blocking_not_checked: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
    NotChecked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub status: CheckStatus,
    pub blocking: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub rule_id: String,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub line: u64,
    pub column: u64,
}

fn deserialize_schema<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    expected: &str,
) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value != expected {
        return Err(serde::de::Error::custom("unsupported content gate schema"));
    }
    Ok(value)
}

fn deserialize_change_schema<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    deserialize_schema(deserializer, CHANGE_SCHEMA)
}

fn deserialize_verdict_schema<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    deserialize_schema(deserializer, VERDICT_SCHEMA)
}

fn deserialize_required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}
