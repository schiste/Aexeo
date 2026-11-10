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
