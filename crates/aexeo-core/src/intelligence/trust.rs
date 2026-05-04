use crate::time_shim::Instant;
use anyhow::{Context, Result, bail};
use csv::ReaderBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use url::Url;

use crate::intelligence::truth::TruthManifest;
use crate::site::{Site, route_from_urlish};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustSurfaceRecord {
    pub source_type: String,
    pub url: String,
    pub title: Option<String>,
    pub snippet: Option<String>,
    pub entity: Option<String>,
    pub observed_at: Option<String>,
    #[serde(default)]
    pub metrics: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustSurfaceSourceSummary {
    pub source_type: String,
    pub rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustSurfaceIssueKind {
    RouteNotInSite,
    MissingCanonicalEntity,
    ForbiddenTerminology,
    DescriptorGap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustSurfaceIssue {
    pub source_type: String,
    pub url: String,
    pub route: Option<String>,
    pub kind: TrustSurfaceIssueKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustSurfaceReconciliation {
    pub rows_read: usize,
    pub matched_first_party_routes: usize,
    pub offsite_mentions: usize,
    pub source_summaries: Vec<TrustSurfaceSourceSummary>,
    pub issues: Vec<TrustSurfaceIssue>,
    pub elapsed_us: u64,
}

pub fn import_trust_surface_records(path: &Path) -> Result<Vec<TrustSurfaceRecord>> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => import_json(path),
        _ => import_csv(path),
    }
}

pub fn reconcile_trust_surfaces(
    records: &[TrustSurfaceRecord],
    site: &Site,
    site_url: Option<&str>,
    manifest: Option<&TruthManifest>,
) -> TrustSurfaceReconciliation {
    let started = Instant::now();
    let canonical_host = site_url
        .and_then(|value| Url::parse(value).ok())
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .or_else(|| {
            manifest
                .and_then(|item| item.organization.as_ref())
                .and_then(|entity| entity.website.as_deref())
                .and_then(|value| Url::parse(value).ok())
                .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        });

    let canonical_labels = canonical_labels(manifest);
    let forbidden_terms = manifest
        .map(|item| item.terminology.forbidden.clone())
        .unwrap_or_default();
    let descriptors = manifest
        .map(|item| {
            item.products
                .iter()
                .flat_map(|product| product.descriptors.iter().cloned())
                .chain(
                    item.organization
                        .iter()
                        .flat_map(|organization| organization.descriptors.iter().cloned()),
                )
                .map(|item| item.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut source_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut matched_first_party_routes = 0;
    let mut offsite_mentions = 0;
    let mut issues = Vec::new();

    for record in records {
        *source_counts.entry(record.source_type.clone()).or_default() += 1;
        let record_host = Url::parse(&record.url)
            .ok()
            .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()));
        let text = [
            record.title.as_deref(),
            record.snippet.as_deref(),
            record.entity.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();

        let route = route_from_urlish(&record.url);
        let first_party = canonical_host
            .as_ref()
            .zip(record_host.as_ref())
            .map(|(left, right)| left == right)
            .unwrap_or(false);
        if first_party {
            if route.as_deref().is_some_and(|route| site.has_route(route)) {
                matched_first_party_routes += 1;
            } else {
                issues.push(TrustSurfaceIssue {
                    source_type: record.source_type.clone(),
                    url: record.url.clone(),
                    route: route.clone(),
                    kind: TrustSurfaceIssueKind::RouteNotInSite,
                    message: format!(
                        "first-party trust surface route {} does not exist in the audited site graph",
                        route.as_deref().unwrap_or("(unresolved)")
                    ),
                });
            }
        } else {
            offsite_mentions += 1;
        }

        if !canonical_labels.is_empty()
            && !canonical_labels
                .iter()
                .any(|label| text.contains(label.as_str()))
        {
            issues.push(TrustSurfaceIssue {
                source_type: record.source_type.clone(),
                url: record.url.clone(),
                route: if first_party { route.clone() } else { None },
                kind: TrustSurfaceIssueKind::MissingCanonicalEntity,
                message: "record does not mention any canonical organization or product label"
                    .to_string(),
            });
        }

        for (forbidden, preferred) in &forbidden_terms {
            if text.contains(&forbidden.to_ascii_lowercase()) {
                issues.push(TrustSurfaceIssue {
                    source_type: record.source_type.clone(),
                    url: record.url.clone(),
                    route: if first_party { route.clone() } else { None },
                    kind: TrustSurfaceIssueKind::ForbiddenTerminology,
                    message: format!(
                        "record uses forbidden term '{}' instead of preferred '{}'",
                        forbidden, preferred
                    ),
                });
            }
        }

        if !descriptors.is_empty()
            && first_party
            && !descriptors
                .iter()
                .any(|descriptor| text.contains(descriptor))
        {
            issues.push(TrustSurfaceIssue {
                source_type: record.source_type.clone(),
                url: record.url.clone(),
                route,
                kind: TrustSurfaceIssueKind::DescriptorGap,
                message: "record does not reflect any canonical product or organization descriptor"
                    .to_string(),
            });
        }
    }

    let mut source_summaries = source_counts
        .into_iter()
        .map(|(source_type, rows)| TrustSurfaceSourceSummary { source_type, rows })
        .collect::<Vec<_>>();
    source_summaries.sort_by(|left, right| left.source_type.cmp(&right.source_type));
    issues.sort_by(|left, right| {
        left.url
            .cmp(&right.url)
            .then_with(|| format!("{:?}", left.kind).cmp(&format!("{:?}", right.kind)))
    });

    TrustSurfaceReconciliation {
        rows_read: records.len(),
        matched_first_party_routes,
        offsite_mentions,
        source_summaries,
        issues,
        elapsed_us: started.elapsed().as_micros() as u64,
    }
}

fn import_csv(path: &Path) -> Result<Vec<TrustSurfaceRecord>> {
    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .from_path(path)
        .with_context(|| format!("failed to read trust surface CSV {}", path.display()))?;
    let headers = reader.headers()?.clone();
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record?;
        let mut metrics = BTreeMap::new();
        let mut source_type = String::new();
        let mut url = String::new();
        let mut title = None;
        let mut snippet = None;
        let mut entity = None;
        let mut observed_at = None;
        for (index, value) in record.iter().enumerate() {
            let Some(header) = headers.get(index) else {
                continue;
            };
            match header {
                "source_type" => source_type = value.to_string(),
                "url" => url = value.to_string(),
                "title" => title = non_empty(value),
                "snippet" => snippet = non_empty(value),
                "entity" => entity = non_empty(value),
                "observed_at" => observed_at = non_empty(value),
                other => {
                    if let Ok(parsed) = value.parse::<u64>() {
                        metrics.insert(other.to_string(), parsed);
                    }
                }
            }
        }
        if source_type.is_empty() || url.is_empty() {
            bail!("trust surface records require source_type and url");
        }
        rows.push(TrustSurfaceRecord {
            source_type,
            url,
            title,
            snippet,
            entity,
            observed_at,
            metrics,
        });
    }
    Ok(rows)
}

fn import_json(path: &Path) -> Result<Vec<TrustSurfaceRecord>> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read trust surface JSON {}", path.display()))?;
    let payload = serde_json::from_str::<Value>(&raw)
        .with_context(|| format!("failed to parse trust surface JSON {}", path.display()))?;
    let rows = match payload {
        Value::Array(items) => items,
        Value::Object(map) => map
            .get("rows")
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default(),
        _ => bail!("trust surface JSON must be an array or an object with a rows field"),
    };

    rows.into_iter()
        .map(|item| {
            let Some(map) = item.as_object() else {
                bail!("trust surface JSON rows must be objects");
            };
            let source_type = map
                .get("source_type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let url = map
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if source_type.is_empty() || url.is_empty() {
                bail!("trust surface records require source_type and url");
            }
            let mut metrics = BTreeMap::new();
            for (key, value) in map {
                if matches!(
                    key.as_str(),
                    "source_type" | "url" | "title" | "snippet" | "entity" | "observed_at"
                ) {
                    continue;
                }
                if let Some(parsed) = value.as_u64() {
                    metrics.insert(key.clone(), parsed);
                }
            }
            Ok(TrustSurfaceRecord {
                source_type,
                url,
                title: map.get("title").and_then(Value::as_str).map(str::to_string),
                snippet: map
                    .get("snippet")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                entity: map
                    .get("entity")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                observed_at: map
                    .get("observed_at")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                metrics,
            })
        })
        .collect()
}

fn non_empty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_string())
}

fn canonical_labels(manifest: Option<&TruthManifest>) -> BTreeSet<String> {
    let mut labels = BTreeSet::new();
    if let Some(manifest) = manifest {
        if let Some(entity) = &manifest.organization {
            labels.insert(entity.name.to_ascii_lowercase());
            labels.extend(entity.aliases.iter().map(|item| item.to_ascii_lowercase()));
        }
        for product in &manifest.products {
            labels.insert(product.name.to_ascii_lowercase());
            labels.extend(product.aliases.iter().map(|item| item.to_ascii_lowercase()));
        }
        labels.extend(
            manifest
                .terminology
                .preferred
                .values()
                .map(|item| item.to_ascii_lowercase()),
        );
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intelligence::truth::{TruthEntity, default_truth_manifest_version};
    use crate::site::load_site;
    use tempfile::tempdir;

    #[test]
    fn imports_trust_surface_csv_records() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("trust.csv");
        fs::write(
            &path,
            "source_type,url,title,snippet,citations\nbing_ai,https://example.com/docs,Docs,Helpful docs,4\n",
        )
        .unwrap();
        let rows = import_trust_surface_records(&path).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].metrics.get("citations"), Some(&4));
    }

    #[test]
    fn reconciles_first_party_route_and_forbidden_terms() {
        let temp = tempdir().unwrap();
        fs::write(
            temp.path().join("index.html"),
            "<html><head><title>Aexeo</title></head><body><h1>Aexeo</h1></body></html>",
        )
        .unwrap();
        let site = load_site(temp.path()).unwrap();
        let manifest = TruthManifest {
            version: default_truth_manifest_version(),
            organization: Some(TruthEntity {
                name: "Aexeo".to_string(),
                aliases: vec![],
                website: Some("https://example.com".to_string()),
                category: None,
                descriptors: vec!["seo".to_string()],
                features: vec![],
            }),
            products: Vec::new(),
            terminology: crate::intelligence::truth::TruthTerminology {
                preferred: BTreeMap::new(),
                forbidden: BTreeMap::from([(
                    "aeo suite".to_string(),
                    "SEO and GEO auditing platform".to_string(),
                )]),
            },
        };
        let report = reconcile_trust_surfaces(
            &[TrustSurfaceRecord {
                source_type: "directory".to_string(),
                url: "https://example.com/missing".to_string(),
                title: Some("AEO suite".to_string()),
                snippet: Some("AEO suite".to_string()),
                entity: None,
                observed_at: None,
                metrics: BTreeMap::new(),
            }],
            &site,
            Some("https://example.com"),
            Some(&manifest),
        );
        assert_eq!(report.rows_read, 1);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == TrustSurfaceIssueKind::RouteNotInSite)
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == TrustSurfaceIssueKind::ForbiddenTerminology)
        );
    }
}
