use seogeo_contracts::Finding;

use crate::config::{Config, SuppressionRule};

fn looks_expired(expires: Option<&str>) -> bool {
    let Some(expires) = expires.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let today = chrono_like_today();
    expires < today.as_str()
}

fn chrono_like_today() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    const SECONDS_PER_DAY: i64 = 86_400;
    let days = seconds / SECONDS_PER_DAY;
    civil_from_days(days)
}

fn civil_from_days(days_since_epoch: i64) -> String {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    format!("{year:04}-{m:02}-{d:02}")
}

fn suppression_matches(rule: &SuppressionRule, finding: &Finding) -> bool {
    if rule.rule_id != finding.rule_id {
        return false;
    }
    let pattern = rule.path_pattern.trim();
    !pattern.is_empty() && finding.path.contains(pattern)
}

pub fn apply_policy(findings: Vec<Finding>, config: &Config) -> Vec<Finding> {
    let policy = config.policy();
    findings
        .into_iter()
        .filter(|finding| {
            !policy
                .ignore_rules
                .iter()
                .any(|rule| rule == &finding.rule_id)
        })
        .filter(|finding| {
            !policy
                .ignore_paths
                .iter()
                .any(|pattern| !pattern.trim().is_empty() && finding.path.contains(pattern))
        })
        .map(|mut finding| {
            if let Some(severity) = policy.severity_overrides.get(&finding.rule_id) {
                finding.severity = severity.clone();
            }
            finding
        })
        .filter(|finding| {
            !policy.suppressions.iter().any(|suppression| {
                !looks_expired(suppression.expires.as_deref())
                    && suppression_matches(suppression, finding)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::apply_policy;
    use crate::config::{Config, SuppressionRule};
    use seogeo_contracts::Finding;

    #[test]
    fn suppresses_matching_active_findings() {
        let findings = vec![Finding {
            rule_id: "SEO001".to_string(),
            message: "missing title".to_string(),
            path: "pages/index.html".to_string(),
            line: 1,
            column: 1,
            severity: "error".to_string(),
            suggestion: None,
        }];
        let config = Config {
            suppressions: vec![SuppressionRule {
                rule_id: "SEO001".to_string(),
                path_pattern: "pages/".to_string(),
                reason: "accepted on legacy pages".to_string(),
                expires: None,
            }],
            ..Config::default()
        };
        assert!(apply_policy(findings, &config).is_empty());
    }

    #[test]
    fn applies_ignore_rules_paths_and_severity_overrides() {
        let findings = vec![
            Finding {
                rule_id: "SEO001".to_string(),
                message: "missing title".to_string(),
                path: "pages/index.html".to_string(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
            },
            Finding {
                rule_id: "SEO002".to_string(),
                message: "missing description".to_string(),
                path: "legacy/about.html".to_string(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
            },
            Finding {
                rule_id: "SEO004".to_string(),
                message: "missing canonical".to_string(),
                path: "pages/about.html".to_string(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
            },
        ];
        let config = Config {
            ignore_rules: vec!["SEO001".to_string()],
            ignore_paths: vec!["legacy/".to_string()],
            severity_overrides: vec![("SEO004".to_string(), "warning".to_string())]
                .into_iter()
                .collect(),
            ..Config::default()
        };

        let filtered = apply_policy(findings, &config);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].rule_id, "SEO004");
        assert_eq!(filtered[0].severity, "warning");
    }
}
