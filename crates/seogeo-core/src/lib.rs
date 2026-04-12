#![forbid(unsafe_code)]

pub mod adapter;
pub mod config;
pub mod content_rules;
pub mod docs;
pub mod fix;
pub mod generate;
pub mod html_rules;
pub mod integrations;
pub mod intelligence;
pub mod link_rules;
pub mod llm_rules;
pub mod plugin;
pub mod policy;
pub mod quality;
pub mod registry;
pub mod reporting;
pub mod robots_rules;
pub mod runtime;
pub mod schema_rules;
pub mod site;
pub mod sitemap_rules;
pub mod social_rules;
pub mod static_check;
pub mod structure_rules;
pub mod verification;

pub use config::{
    Config, ConfigFieldDoc, render_resolved_config_json, render_resolved_config_toml,
};
pub use content_rules::run_content_rules;
pub use docs::{
    find_reference_doc_drift, reference_documents, render_adapter_reference,
    render_config_reference, render_rule_reference, write_reference_documents,
};
pub use fix::apply_safe_fixes;
pub use generate::{
    build_link_suggestions, render_llms_full_txt, render_llms_txt, render_markdown_mirror,
    render_robots_txt, suggest_internal_links,
};
pub use html_rules::{run_html_rules, run_static_html_audit};
pub use integrations::{
    BingAiImportReport, BingAiOpportunity, BingAiOpportunityReport, BingAiRecord, BingAiTrendDelta,
    BingAiTrendReport, BingAiTrendRoute, BingAiTrendSnapshot, BingAiUrlSummary, IndexNowLedger,
    IndexNowLedgerEntry, IndexNowRetryReport, IndexNowSubmission, IndexNowValidation,
    PublishHookReport, SearchConsoleExportRow, SnippetInspection, build_bing_ai_opportunity_report,
    build_bing_ai_trend_report, build_publish_hook_report, build_publish_hook_report_with_config,
    export_search_console_rows, import_bing_ai_export, inspect_snippet_controls_path,
    inspect_snippet_controls_site, inspect_snippet_controls_url,
    inspect_snippet_controls_with_config, load_bing_ai_trends, load_indexnow_ledger,
    record_bing_ai_trend, retry_indexnow_submissions, submit_indexnow, submit_indexnow_with_ledger,
    validate_indexnow,
};
pub use intelligence::{
    GroundingCoverageGap, GroundingIntentFamily, GroundingRouteAnalysis, GroundingSiteAnalysis,
    TrustSurfaceIssue, TrustSurfaceIssueKind, TrustSurfaceReconciliation, TrustSurfaceRecord,
    TrustSurfaceSourceSummary, TruthAssessment, TruthEntity, TruthManifest, TruthMismatch,
    TruthMismatchSeverity, TruthStructuredSource, assess_truth_layer, discover_truth_manifest,
    import_trust_surface_records, load_truth_manifest, map_grounding_queries,
    reconcile_trust_surfaces,
};
pub use link_rules::run_link_rules;
pub use llm_rules::run_llm_rules;
pub use plugin::{PluginManifestCheck, validate_python_plugin_module};
pub use policy::apply_policy;
pub use quality::{QUALITY_RULES, run_repo_quality_checks};
pub use registry::{
    AdapterDefinition, RuleDescriptor, RuleGroupDefinition, builtin_adapters, builtin_rule_groups,
    list_adapter_names, list_rule_group_names,
};
pub use reporting::{
    DEFAULT_AUDIT_LOG_LIMIT, build_audit_artifact, build_recap_lines, render_audit_artifact_json,
    render_json, render_markdown_artifact, render_sarif, render_text, render_text_artifact,
    rule_group_name, summarize_findings, write_audit_artifact, write_partial_audit_artifact,
    write_progress_audit_artifact,
};
pub use robots_rules::run_robots_rules;
pub use runtime::{
    PlaywrightDoctor, RuntimeAudit, RuntimeAuditOptions, RuntimeProgressEvent, RuntimeProgressMode,
    run_runtime_audit, run_runtime_audit_with_options, runtime_doctor, verify_runtime_audit,
};
pub use schema_rules::run_schema_rules;
pub use site::{
    AlternateLink, Link, Page, PageKind, Site, load_site, normalize_internal_href,
    route_from_urlish,
};
pub use sitemap_rules::run_sitemap_rules;
pub use social_rules::run_social_rules;
pub use static_check::{
    can_run_native_static_audit, run_native_static_audit, run_native_static_audit_with_config,
};
pub use structure_rules::run_structure_rules;
pub use verification::{
    DiffResult, diff_finding_sets, load_audit_artifact, load_findings_from_audit, render_diff_text,
    write_baseline_file,
};
