//! Experimental, offline candidate link-impact query.
use super::common::required_arg;
use aexeo_contracts::{
    content_gate_change_validation::{parse_strict_validated_change, parse_unique_json_value},
    content_gate_protocol::Change,
};
use aexeo_core::content_candidate::{
    CandidateItem, CandidateSnapshot, RemovedRouteReferences, TransitiveLinkImpact,
    compare_removed_route_references, overlay_candidate,
    overlay_candidate_with_transitive_link_impact,
};
use aexeo_core::content_candidate_profile::candidate_revision_digest;
use anyhow::{Result, bail};
use clap::ArgMatches;
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read};

const MAX_QUERY_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    schema: String,
    snapshot: CandidateSnapshot,
    change: Change,
    after: Vec<CandidateItem>,
}
#[derive(Serialize)]
struct Response {
    schema: &'static str,
    status: &'static str,
    reason: Option<String>,
    change_id: Option<String>,
    impact: Option<aexeo_core::content_candidate::CandidateImpact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transitive_link_impact: Option<TransitiveLinkImpact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    removed_route_references: Option<RemovedRouteReferences>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_revision_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate: Option<CandidateSnapshot>,
}
fn emit(response: &Response) -> Result<()> {
    println!("{}", serde_json::to_string(response)?);
    Ok(())
}
pub fn command_content_impact(args: &ArgMatches) -> Result<i32> {
    let started = std::time::Instant::now();
    let path = required_arg(args, "input")?;
    let mut data = Vec::new();
    File::open(path)?
        .take(MAX_QUERY_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_QUERY_BYTES {
        bail!("impact query exceeds size limit");
    }
    let value = parse_unique_json_value(&data)?;
    let change = value
        .get("change")
        .ok_or_else(|| anyhow::anyhow!("missing change"))?;
    parse_strict_validated_change(&serde_json::to_vec(change)?)?;
    let query: Query = serde_json::from_value(value)?;
    if query.schema != "aexeo.content-impact-query/v1-experimental" {
        bail!("unsupported impact query schema");
    }
    let transitive = args.get_flag("transitive");
    let removed_routes = args.get_flag("removed-route-references");
    let result = if transitive {
        overlay_candidate_with_transitive_link_impact(&query.snapshot, &query.change, &query.after)
            .map(|(overlay, impact)| (overlay, Some(impact)))
    } else {
        overlay_candidate(&query.snapshot, &query.change, &query.after)
            .map(|overlay| (overlay, None))
    }
    .and_then(|(overlay, impact)| {
        let revision = candidate_revision_digest(&overlay.candidate)?;
        let references = if removed_routes {
            Some(compare_removed_route_references(
                &query.snapshot,
                &overlay.candidate,
            )?)
        } else {
            None
        };
        Ok((overlay, impact, references, revision))
    });
    let response = match result {
        Ok((result, transitive_link_impact, removed_route_references, revision)) => {
            let status = if result.impact.link_impact_complete
                && removed_route_references
                    .as_ref()
                    .is_none_or(|report| report.complete)
            {
                "complete"
            } else {
                "not_checked"
            };
            Response {
                schema: "aexeo.content-impact-result/v1-experimental",
                status,
                reason: None,
                change_id: Some(result.change_id),
                impact: Some(result.impact),
                transitive_link_impact,
                removed_route_references,
                candidate_revision_sha256: Some(
                    revision.iter().map(|b| format!("{b:02x}")).collect(),
                ),
                candidate: args
                    .get_flag("include-candidate")
                    .then_some(result.candidate),
            }
        }
        Err(reason) => Response {
            schema: "aexeo.content-impact-result/v1-experimental",
            status: "not_checked",
            reason: Some(reason.into()),
            change_id: Some(query.change.change_id),
            impact: None,
            transitive_link_impact: None,
            removed_route_references: None,
            candidate_revision_sha256: None,
            candidate: None,
        },
    };
    let code = if response.status == "complete" { 0 } else { 2 };
    if args.get_flag("telemetry") {
        let mut telemetry = serde_json::json!({"schema":"aexeo.content-impact-telemetry/v1", "duration_ms":started.elapsed().as_millis(), "query_bytes":data.len(), "status":response.status, "affected_count":response.impact.as_ref().map(|i| i.affected.len())});
        if transitive {
            telemetry["transitive_affected_count"] = serde_json::json!(
                response
                    .transitive_link_impact
                    .as_ref()
                    .map(|i| i.affected.len())
            );
        }
        if removed_routes {
            telemetry["removed_route_count"] = serde_json::json!(
                response
                    .removed_route_references
                    .as_ref()
                    .map(|r| r.removed_routes.len())
            );
            telemetry["removed_route_reference_count"] = serde_json::json!(
                response
                    .removed_route_references
                    .as_ref()
                    .map(|r| r.references.len())
            );
        }
        eprintln!("{telemetry}");
    }
    emit(&response)?;
    Ok(code)
}
