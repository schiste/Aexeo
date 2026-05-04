use aexeo_contracts::Finding;

use crate::config::{Config, RouteKind, SuppressionRule};

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
        .filter(|finding| !route_kind_skips_finding(&config.route_kinds, finding))
        .collect()
}

/// Returns true when any route kind matches this finding's path
/// AND lists the finding's rule_id in its skip_rules. Compiled
/// inline rather than expanded into a Vec<SuppressionRule>
/// upfront — saves an allocation per audit and keeps the
/// route-kind name available for future provenance reporting.
fn route_kind_skips_finding(
    route_kinds: &std::collections::BTreeMap<String, RouteKind>,
    finding: &Finding,
) -> bool {
    for kind in route_kinds.values() {
        if !kind.skip_rules.iter().any(|rule| rule == &finding.rule_id) {
            continue;
        }
        if kind
            .r#match
            .iter()
            .any(|pattern| !pattern.trim().is_empty() && finding.path.contains(pattern))
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::apply_policy;
    use crate::config::{Config, RouteKind, SuppressionRule};
    use aexeo_contracts::{Finding, FindingScope};

    #[test]
    fn route_kind_skips_listed_rules_on_matching_paths() {
        // Aeptus's request: declaring [route_kinds.manifesto]
        // with match=["/foundations/"] and
        // skip_rules=["GEO007","GEO008","GEO010"] should silence
        // those rules on /foundations/* without scattering one
        // suppression entry per rule.
        let findings = vec![
            Finding {
                rule_id: "GEO007".to_string(),
                message: "thin content".to_string(),
                path: "/foundations/principles".to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                suggestion: None,
                scope: FindingScope::Page,
            },
            Finding {
                rule_id: "GEO007".to_string(),
                message: "thin content".to_string(),
                path: "/blog/some-post".to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                suggestion: None,
                scope: FindingScope::Page,
            },
        ];
        let mut route_kinds = BTreeMap::new();
        route_kinds.insert(
            "manifesto".to_string(),
            RouteKind {
                r#match: vec!["/foundations/".to_string()],
                skip_rules: vec!["GEO007".to_string(), "GEO008".to_string()],
                noindex: false,
            },
        );
        let config = Config {
            route_kinds,
            ..Config::default()
        };
        let surviving = apply_policy(findings, &config);
        assert_eq!(
            surviving.len(),
            1,
            "blog post finding survives, manifesto's gets skipped: {surviving:?}"
        );
        assert_eq!(surviving[0].path, "/blog/some-post");
    }

    #[test]
    fn route_kind_does_not_skip_unlisted_rules() {
        let findings = vec![Finding {
            rule_id: "SEO002".to_string(),
            message: "missing description".to_string(),
            path: "/foundations/principles".to_string(),
            line: 1,
            column: 1,
            severity: "warning".to_string(),
            suggestion: None,
            scope: FindingScope::Page,
        }];
        let mut route_kinds = BTreeMap::new();
        route_kinds.insert(
            "manifesto".to_string(),
            RouteKind {
                r#match: vec!["/foundations/".to_string()],
                skip_rules: vec!["GEO007".to_string()],
                noindex: false,
            },
        );
        let config = Config {
            route_kinds,
            ..Config::default()
        };
        // SEO002 isn't in skip_rules, so the finding survives.
        let surviving = apply_policy(findings, &config);
        assert_eq!(surviving.len(), 1);
    }

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
            scope: FindingScope::Page,
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
                scope: FindingScope::Page,
            },
            Finding {
                rule_id: "SEO002".to_string(),
                message: "missing description".to_string(),
                path: "legacy/about.html".to_string(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
                scope: FindingScope::Page,
            },
            Finding {
                rule_id: "SEO004".to_string(),
                message: "missing canonical".to_string(),
                path: "pages/about.html".to_string(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
                scope: FindingScope::Page,
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
