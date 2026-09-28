use aexeo_contracts::{Finding, FindingScope};
use std::fs;
use std::path::Path;

pub(super) fn find_static_tooling_issues(root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (rule_id, filename, message) in [
        ("QLT009", "Cargo.toml", "missing Cargo workspace manifest"),
        (
            "QLT010",
            "scripts/build-rust.sh",
            "missing Rust build script",
        ),
        (
            "QLT011",
            "performance-budget.json",
            "missing performance budget file",
        ),
        (
            "QLT013",
            "scripts/install-aexeo.sh",
            "missing install script",
        ),
        ("QLT021", "deny.toml", "missing cargo-deny policy file"),
        ("QLT014", "scripts/ci-local.sh", "missing local CI script"),
        (
            "QLT015",
            "scripts/install-hooks.sh",
            "missing git hook installation script",
        ),
        (
            "QLT022",
            "scripts/check-deps.sh",
            "missing dependency hygiene script",
        ),
        ("QLT016", ".githooks/pre-commit", "missing pre-commit hook"),
        ("QLT017", ".githooks/pre-push", "missing pre-push hook"),
    ] {
        if !root.join(filename).exists() {
            findings.push(Finding {
                rule_id: rule_id.to_string(),
                message: message.to_string(),
                path: filename.to_string(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
                scope: FindingScope::Sitewide,
            });
        }
    }
    if root.join("package.json").exists() && !root.join("package-lock.json").exists() {
        findings.push(Finding {
            rule_id: "QLT023".to_string(),
            message: "missing Node package lockfile for browser runtime".to_string(),
            path: "package-lock.json".to_string(),
            line: 1,
            column: 1,
            severity: "error".to_string(),
            suggestion: Some("run `npm install` in the repository root to capture a reproducible Playwright runtime".to_string()),
            scope: FindingScope::Sitewide,
        });
    }
    findings
}

/// A config key that is declared, documented, and loaded — but never read by
/// any code path.
///
/// This shipped for a long time. `coverage_threshold`, `complexity_threshold`,
/// `typecheck_command`, `enable_cache`, `cache_dir`, and `cache_ttl_seconds`
/// were all documented in `SPEC.md` and `docs/config.md`, all accepted by the
/// loader, all rendered by `config print`, and all did nothing. Setting
/// `enable_cache = false` changed no behaviour because there was no cache;
/// there was no coverage tooling to compare `coverage_threshold` against. The
/// root `.coveragerc` that appeared to enforce it was a Python leftover
/// pointing at a directory that no longer existed.
///
/// Dead configuration is worse than absent configuration: it advertises a
/// guarantee the tool does not provide, and a user who sets it reasonably
/// assumes something is enforcing it.
///
/// The check is deliberately narrow. A key must be referenced by the
/// `config.rs` field list *and* mentioned nowhere else in `src/`, so a key
/// that is merely declared but used through a different spelling is not
/// falsely reported. A key that is only forwarded into a view struct
/// (`views.rs`) still counts as dead, which is the case that let the cache
/// keys through before.
pub(super) fn find_unread_config_keys(root: &Path) -> Vec<Finding> {
    let config_rs = root.join("crates/aexeo-core/src/config.rs");
    let Ok(config_source) = fs::read_to_string(&config_rs) else {
        return Vec::new();
    };

    // `    pub <name>: <type>,` inside the `Config` struct.
    let declared = config_source
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed.strip_prefix("pub ")?;
            let (name, tail) = rest.split_once(':')?;
            if !tail
                .trim_start()
                .starts_with(|c: char| c.is_ascii_uppercase())
            {
                return None;
            }
            let valid = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            valid.then_some(name.to_string())
        })
        .collect::<Vec<_>>();

    if declared.is_empty() {
        return Vec::new();
    }

    let mut unread = Vec::new();
    for key in declared {
        if config_key_is_referenced(root, &key) {
            continue;
        }
        unread.push(Finding {
            rule_id: "QLT024".to_string(),
            message: format!(
                "config key `{key}` is declared and documented but never read; it does nothing"
            ),
            path: "crates/aexeo-core/src/config.rs".to_string(),
            line: 1,
            column: 1,
            severity: "error".to_string(),
            suggestion: Some(format!(
                "either read `{key}` where it matters, or remove the field, its \
                 default, its doc entry, and its schema property"
            )),
            scope: FindingScope::Sitewide,
        });
    }
    unread
}

/// Whether `key` appears anywhere in `src/` other than the bare field
/// declaration in `config.rs`.
///
/// `config.rs` is excluded because the field declaration itself would
/// otherwise be the only match and every key would look read.
fn config_key_is_referenced(root: &Path, key: &str) -> bool {
    let src = root.join("crates/aexeo-core/src");
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) == Some("config.rs")
                && path.parent().map(|p| p.ends_with("aexeo-core/src")) == Some(true)
            {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if text.contains(key) {
                return true;
            }
        }
    }
    false
}

pub(super) fn find_missing_rust_integration_coverage(root: &Path) -> Vec<Finding> {
    let tests_dir = root.join("crates/aexeo-cli/tests");
    if tests_dir.exists()
        && fs::read_dir(&tests_dir)
            .ok()
            .into_iter()
            .flat_map(|entries| entries.filter_map(std::result::Result::ok))
            .map(|entry| entry.path())
            .any(|path| {
                path.extension().and_then(|ext| ext.to_str()) == Some("rs")
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.ends_with("_integration.rs"))
            })
    {
        return Vec::new();
    }
    vec![Finding {
        rule_id: "QLT012".to_string(),
        message: "missing Rust CLI integration test coverage".to_string(),
        path: "crates/aexeo-cli/tests".to_string(),
        line: 1,
        column: 1,
        severity: "error".to_string(),
        suggestion: None,
        scope: FindingScope::Sitewide,
    }]
}
