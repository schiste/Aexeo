//! Structural checks for content-gate change documents.
//!
//! This module does not read content, validate timestamps or URIs, check
//! digests against bytes, resolve symlinks, or prove subject binding. Callers
//! must perform those checks before evaluating or trusting a change.

use crate::content_gate_protocol::{
    BaseKind, CHANGE_SCHEMA, Change, ContentState, Operation, OriginKind,
};
use std::{collections::BTreeSet, fmt, path::Path};

/// A structural rejection. Messages never include document content or paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeValidationError(pub &'static str);

impl fmt::Display for ChangeValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ChangeValidationError {}

/// Parse and validate a v1 change document before evaluation. Unknown fields
/// remain permitted for forward compatibility. No content is read or fetched.
pub fn parse_validated_change(bytes: &[u8]) -> Result<Change, ChangeValidationError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| ChangeValidationError("invalid change JSON"))?;
    validate_present_fields(&value)?;
    let change: Change = serde_json::from_value(value)
        .map_err(|_| ChangeValidationError("invalid change fields"))?;
    validate_change_structure(&change)?;
    if chrono::DateTime::parse_from_rfc3339(&change.created_at).is_err() {
        return Err(ChangeValidationError("invalid change timestamp"));
    }
    if change.content_root.as_ref().is_some_and(String::is_empty)
        || change.origin.system.as_ref().is_some_and(String::is_empty)
        || change
            .origin
            .reference
            .as_ref()
            .is_some_and(String::is_empty)
        || change
            .author
            .as_ref()
            .and_then(|author| author.id.as_ref())
            .is_some_and(String::is_empty)
    {
        return Err(ChangeValidationError("empty optional field"));
    }
    if let Some(site) = &change.site
        && (site.id.as_ref().is_some_and(String::is_empty)
            || site.base_url.as_ref().is_some_and(|uri| !valid_uri(uri)))
    {
        return Err(ChangeValidationError("invalid site field"));
    }
    for entry in &change.entries {
        if entry
            .previous_entity_id
            .as_ref()
            .is_some_and(String::is_empty)
        {
            return Err(ChangeValidationError("empty previous identity"));
        }
        for state in [entry.before.as_ref(), entry.after.as_ref()]
            .into_iter()
            .flatten()
        {
            if state.revision.as_ref().is_some_and(String::is_empty)
                || state.uri.as_ref().is_some_and(|uri| !valid_uri(uri))
            {
                return Err(ChangeValidationError("invalid content state field"));
            }
        }
    }
    Ok(change)
}

/// Decode JSON without discarding duplicate keys or fields unknown to `T`.
/// Callers must bound the input before calling this function. This is intended
/// for strict, versioned bindings; the ordinary v1 parser remains permissive.
pub fn parse_lossless_json<T>(bytes: &[u8]) -> Result<T, ChangeValidationError>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let value = parse_unique_json_value(bytes)?;
    let decoded: T = serde_json::from_value(value.clone())
        .map_err(|_| ChangeValidationError("invalid JSON fields"))?;
    if serde_json::to_value(&decoded).map_err(|_| ChangeValidationError("invalid JSON fields"))?
        != value
    {
        return Err(ChangeValidationError("unrepresented JSON field or value"));
    }
    Ok(decoded)
}

/// Parse JSON while rejecting duplicate keys at every depth. Callers must
/// bound the input and validate its shape and semantics independently.
pub fn parse_unique_json_value(bytes: &[u8]) -> Result<serde_json::Value, ChangeValidationError> {
    let UniqueJson(value) = serde_json::from_slice(bytes)
        .map_err(|_| ChangeValidationError("invalid or duplicate JSON field"))?;
    Ok(value)
}

/// Strict variant for binding a complete v1 change in an experimental receipt.
pub fn parse_strict_validated_change(bytes: &[u8]) -> Result<Change, ChangeValidationError> {
    let _: Change = parse_lossless_json(bytes)?;
    parse_validated_change(bytes)
}

struct UniqueJson(serde_json::Value);

impl<'de> serde::Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{Error, MapAccess, SeqAccess, Visitor};
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_i64<E: Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_u64<E: Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_f64<E: Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| UniqueJson(n.into()))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_string<E: Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(serde_json::Value::Null))
            }
            fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(serde_json::Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueJson(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(UniqueJson(values.into()))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(A::Error::custom("duplicate JSON key"));
                    }
                    let UniqueJson(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(UniqueJson(values.into()))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

fn valid_uri(value: &str) -> bool {
    iri_string::types::UriStr::new(value).is_ok()
}

// Serde treats `null` in an Option as absence, but the v1 schema allows
// omission, not null, for each of these known fields.
fn validate_present_fields(value: &serde_json::Value) -> Result<(), ChangeValidationError> {
    fn reject_null(
        value: &serde_json::Value,
        fields: &[&str],
    ) -> Result<(), ChangeValidationError> {
        if fields
            .iter()
            .any(|field| value.get(*field).is_some_and(serde_json::Value::is_null))
        {
            return Err(ChangeValidationError("null change field"));
        }
        Ok(())
    }
    reject_null(value, &["content_root", "site", "author", "extensions"])?;
    if let Some(origin) = value.get("origin") {
        reject_null(origin, &["system", "ref"])?;
    }
    if let Some(site) = value.get("site") {
        if site.as_object().is_some_and(serde_json::Map::is_empty) {
            return Err(ChangeValidationError("empty site"));
        }
        reject_null(site, &["id", "base_url"])?;
    }
    if let Some(author) = value.get("author") {
        reject_null(author, &["id", "display_name", "kind"])?;
    }
    if let Some(entries) = value.get("entries").and_then(serde_json::Value::as_array) {
        for entry in entries {
            reject_null(
                entry,
                &[
                    "previous_entity_id",
                    "public_path",
                    "metadata",
                    "before",
                    "after",
                ],
            )?;
            for state in [entry.get("before"), entry.get("after")]
                .into_iter()
                .flatten()
            {
                reject_null(state, &["revision", "inline", "file", "uri"])?;
            }
        }
    }
    Ok(())
}

/// Validate a subset of v1's structural invariants, without accessing sources.
pub fn validate_change_structure(change: &Change) -> Result<(), ChangeValidationError> {
    if change.schema != CHANGE_SCHEMA {
        return Err(ChangeValidationError("unsupported change schema"));
    }
    if change.change_id.is_empty()
        || change.created_at.is_empty()
        || change.base.revision.is_empty()
    {
        return Err(ChangeValidationError("required change field is empty"));
    }
    if matches!(change.base.kind, BaseKind::Git) && !git_object_id(&change.base.revision) {
        return Err(ChangeValidationError("invalid git base revision"));
    }
    if matches!(change.origin.kind, OriginKind::Git) && !matches!(change.base.kind, BaseKind::Git) {
        return Err(ChangeValidationError("git origin requires a git base"));
    }
    if change.entries.is_empty() {
        return Err(ChangeValidationError("change contains no entries"));
    }
    let mut ids = BTreeSet::new();
    for entry in &change.entries {
        if entry.entity_id.is_empty()
            || entry.media_type.is_empty()
            || !ids.insert(&entry.entity_id)
        {
            return Err(ChangeValidationError(
                "entry identity or media type is invalid",
            ));
        }
        if entry
            .public_path
            .as_ref()
            .is_some_and(|path| !path.starts_with('/'))
        {
            return Err(ChangeValidationError("invalid public path"));
        }
        match entry.operation {
            Operation::Create if entry.before.is_some() || entry.after.is_none() => {
                return Err(ChangeValidationError("invalid create states"));
            }
            Operation::Update if entry.after.is_none() => {
                return Err(ChangeValidationError("update requires after state"));
            }
            Operation::Delete if entry.after.is_some() => {
                return Err(ChangeValidationError("delete cannot have after state"));
            }
            Operation::Rename
                if entry.after.is_none()
                    || entry
                        .previous_entity_id
                        .as_deref()
                        .is_none_or(str::is_empty) =>
            {
                return Err(ChangeValidationError(
                    "rename requires previous identity and after state",
                ));
            }
            _ => {}
        }
        for state in [entry.before.as_ref(), entry.after.as_ref()]
            .into_iter()
            .flatten()
        {
            validate_state(state, change.content_root.as_deref())?;
        }
    }
    Ok(())
}

fn git_object_id(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64) && lowercase_hex(value)
}

fn lowercase_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn validate_state(state: &ContentState, root: Option<&str>) -> Result<(), ChangeValidationError> {
    if state.sha256.len() != 64 || !lowercase_hex(&state.sha256) {
        return Err(ChangeValidationError("invalid content digest"));
    }
    if [
        state.inline.is_some(),
        state.file.is_some(),
        state.uri.is_some(),
    ]
    .into_iter()
    .filter(|v| *v)
    .count()
        != 1
    {
        return Err(ChangeValidationError(
            "content state requires exactly one source",
        ));
    }
    if let Some(path) = &state.file {
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path.split('/').any(|part| part == "..")
        {
            return Err(ChangeValidationError("invalid content file path"));
        }
        if !root.is_some_and(|value| Path::new(value).is_absolute()) {
            return Err(ChangeValidationError(
                "content file requires an absolute content root",
            ));
        }
    }
    if state.uri.as_ref().is_some_and(String::is_empty) {
        return Err(ChangeValidationError("empty content URI"));
    }
    Ok(())
}

#[cfg(test)]
mod strict_json_tests {
    use super::*;

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Example {
        payload: serde_json::Value,
    }

    #[test]
    fn strict_json_rejects_duplicates_at_all_depths_and_unrepresented_fields() {
        let valid = br#"{"payload":{"array":[null,true,0,-2,1.5,"text",{"x":1}]}}"#;
        assert!(parse_lossless_json::<Example>(valid).is_ok());
        for invalid in [
            br#"{"payload":null,"payload":null}"#.as_slice(),
            br#"{"payload":{"x":1,"x":2}}"#.as_slice(),
            br#"{"payload":[{"x":1,"x":1}]}"#.as_slice(),
            br#"{"payload":null,"unknown":1}"#.as_slice(),
        ] {
            assert!(parse_lossless_json::<Example>(invalid).is_err());
        }
    }
}
