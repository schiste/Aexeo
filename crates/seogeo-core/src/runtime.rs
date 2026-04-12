mod fetcher;
mod graph;
mod http;
mod planner;
mod playwright;
mod snapshot;

use anyhow::Result;
use seogeo_contracts::{
    AuditArtifact, AuditPerformance, AuditStatus, CrawlStats, Finding, FindingScope, PhaseTiming,
    SlowCrawlPath,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::Instant;

use crate::config::Config;
use crate::reporting::build_audit_artifact;
use crate::site::{Site, route_from_urlish};
use crate::static_check::run_checks_for_site_profiled;
use crate::verification::{DiffResult, diff_finding_sets};
use fetcher::{FetchOutcome, RuntimeFetcher};
use graph::{extract_internal_links, read_loc_values, response_report_path, should_enqueue_link};
use http::{fetch_with_http, host_for_url, is_html_content_type, same_site_host};
use planner::{CrawlPlanner, CrawlPlannerState};
pub use playwright::PlaywrightDoctor;
use playwright::{playwright_is_available, probe_playwright_runtime};
use snapshot::{RuntimeSnapshotBuilder, RuntimeSnapshotState};

#[derive(Debug, Clone)]
pub struct RuntimeAudit {
    pub site: Site,
    pub crawl_findings: Vec<Finding>,
    pub findings: Vec<Finding>,
    pub status: AuditStatus,
    pub crawl_stats: CrawlStats,
    pub truncation_reason: Option<String>,
    pub performance: Option<AuditPerformance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeCheckpoint {
    pub(crate) version: u32,
    pub(crate) base_url: String,
    pub(crate) engine: String,
    pub(crate) planner: CrawlPlannerState,
    pub(crate) snapshot: RuntimeSnapshotState,
    pub(crate) crawl_findings: Vec<Finding>,
    pub(crate) crawl_stats: CrawlStats,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeProgressEvent {
    pub phase: String,
    pub engine: String,
    pub current_url: Option<String>,
    pub visited_pages: usize,
    pub discovered_internal_routes: usize,
    pub queued_routes_remaining: usize,
    pub fetch_failures: usize,
    pub fetch_retries: usize,
    pub skipped_non_html: usize,
    pub truncated: bool,
    pub elapsed_ms: u64,
    pub elapsed_us: u64,
    pub pages_per_minute: usize,
    pub average_fetch_ms: u64,
    pub average_page_process_ms: u64,
    pub average_partial_audit_ms: u64,
    pub checkpoints_written: usize,
    pub progress_artifacts_written: usize,
    pub partial_artifacts_written: usize,
}

pub struct RuntimeAuditOptions<'a> {
    pub checkpoint_path: Option<&'a Path>,
    pub checkpoint_every: usize,
    pub progress_artifact_every: usize,
    pub progress_artifact_min_interval_ms: u64,
    pub partial_audit_every: usize,
    pub partial_audit_min_interval_ms: u64,
    pub resume_from: Option<&'a Path>,
    pub fetch_retry_budget: usize,
    pub progress: RuntimeProgressMode<'a>,
    pub artifact_command: &'a str,
    pub progress_artifact: RuntimeArtifactMode<'a>,
    pub partial_audit_artifact: RuntimeArtifactMode<'a>,
}

#[derive(Default)]
pub enum RuntimeProgressMode<'a> {
    #[default]
    Off,
    Callback(&'a mut dyn FnMut(RuntimeProgressEvent)),
}

#[derive(Default)]
pub enum RuntimeArtifactMode<'a> {
    #[default]
    Off,
    Callback(&'a mut dyn FnMut(&AuditArtifact) -> Result<()>),
}

impl<'a> Default for RuntimeAuditOptions<'a> {
    fn default() -> Self {
        Self {
            checkpoint_path: None,
            checkpoint_every: 25,
            progress_artifact_every: 10,
            progress_artifact_min_interval_ms: 5_000,
            partial_audit_every: 50,
            partial_audit_min_interval_ms: 30_000,
            resume_from: None,
            fetch_retry_budget: 2,
            progress: RuntimeProgressMode::Off,
            artifact_command: "crawl",
            progress_artifact: RuntimeArtifactMode::Off,
            partial_audit_artifact: RuntimeArtifactMode::Off,
        }
    }
}

impl RuntimeAudit {
    pub fn is_partial(&self) -> bool {
        self.status == AuditStatus::Partial
    }
}

#[derive(Debug, Clone, Default)]
struct RuntimePerformance {
    started_at: Option<Instant>,
    total_fetch_us: u64,
    total_page_process_us: u64,
    total_snapshot_write_us: u64,
    total_queue_selection_us: u64,
    total_planner_update_us: u64,
    total_link_extraction_us: u64,
    total_progress_callback_us: u64,
    total_checkpoint_write_us: u64,
    total_progress_artifact_write_us: u64,
    total_sitemap_seed_us: u64,
    total_optional_artifact_fetch_us: u64,
    total_snapshot_build_us: u64,
    total_partial_audit_us: u64,
    total_partial_audit_build_us: u64,
    total_partial_artifact_write_us: u64,
    total_final_audit_us: u64,
    total_rule_evaluation_us: u64,
    total_policy_apply_us: u64,
    partial_audits_built: usize,
    checkpoints_written: usize,
    progress_artifacts_written: usize,
    partial_artifacts_written: usize,
    last_progress_emit_at: Option<Instant>,
    last_progress_emit_page: usize,
    last_partial_emit_at: Option<Instant>,
    last_partial_emit_page: usize,
    slowest_paths: Vec<SlowCrawlPath>,
}

impl RuntimePerformance {
    fn new() -> Self {
        Self {
            started_at: Some(Instant::now()),
            ..Self::default()
        }
    }

    fn from_started_elapsed(elapsed_us: u64) -> Self {
        let mut performance = Self::new();
        if elapsed_us > 0 {
            performance.started_at =
                Some(Instant::now() - std::time::Duration::from_micros(elapsed_us));
        }
        performance
    }

    fn record_fetch(&mut self, duration_us: u64) {
        self.total_fetch_us = self.total_fetch_us.saturating_add(duration_us);
    }

    fn record_page_process(&mut self, url: &str, fetch_us: u64, process_us: u64) {
        self.total_page_process_us = self.total_page_process_us.saturating_add(process_us);
        self.slowest_paths.push(SlowCrawlPath {
            url: url.to_string(),
            fetch_us,
            process_us,
            fetch_ms: fetch_us / 1_000,
            process_ms: process_us / 1_000,
        });
        self.slowest_paths.sort_by(|left, right| {
            let left_total = left.fetch_us.saturating_add(left.process_us);
            let right_total = right.fetch_us.saturating_add(right.process_us);
            right_total
                .cmp(&left_total)
                .then_with(|| left.url.cmp(&right.url))
        });
        self.slowest_paths.truncate(5);
    }

    fn record_queue_selection(&mut self, duration_us: u64) {
        self.total_queue_selection_us = self.total_queue_selection_us.saturating_add(duration_us);
    }

    fn record_snapshot_write(&mut self, duration_us: u64) {
        self.total_snapshot_write_us = self.total_snapshot_write_us.saturating_add(duration_us);
    }

    fn record_planner_update(&mut self, duration_us: u64) {
        self.total_planner_update_us = self.total_planner_update_us.saturating_add(duration_us);
    }

    fn record_link_extraction(&mut self, duration_us: u64) {
        self.total_link_extraction_us = self.total_link_extraction_us.saturating_add(duration_us);
    }

    fn record_progress_callback(&mut self, duration_us: u64) {
        self.total_progress_callback_us =
            self.total_progress_callback_us.saturating_add(duration_us);
    }

    fn record_checkpoint_write(&mut self, duration_us: u64) {
        self.total_checkpoint_write_us = self.total_checkpoint_write_us.saturating_add(duration_us);
    }

    fn record_progress_artifact_write(&mut self, duration_us: u64) {
        self.total_progress_artifact_write_us = self
            .total_progress_artifact_write_us
            .saturating_add(duration_us);
    }

    fn record_optional_artifact_fetch(&mut self, duration_us: u64) {
        self.total_optional_artifact_fetch_us = self
            .total_optional_artifact_fetch_us
            .saturating_add(duration_us);
    }

    fn record_sitemap_seed(&mut self, duration_us: u64) {
        self.total_sitemap_seed_us = self.total_sitemap_seed_us.saturating_add(duration_us);
    }

    fn record_snapshot_build(&mut self, duration_us: u64) {
        self.total_snapshot_build_us = self.total_snapshot_build_us.saturating_add(duration_us);
    }

    fn record_final_audit(&mut self, duration_us: u64) {
        self.total_final_audit_us = self.total_final_audit_us.saturating_add(duration_us);
    }

    fn record_rule_evaluation(&mut self, duration_us: u64) {
        self.total_rule_evaluation_us = self.total_rule_evaluation_us.saturating_add(duration_us);
    }

    fn record_policy_apply(&mut self, duration_us: u64) {
        self.total_policy_apply_us = self.total_policy_apply_us.saturating_add(duration_us);
    }

    fn record_checkpoint(&mut self) {
        self.checkpoints_written += 1;
    }

    fn should_emit_progress_artifact(
        &self,
        visited_pages: usize,
        options: &RuntimeAuditOptions<'_>,
    ) -> bool {
        if visited_pages == 0 {
            return false;
        }
        if self.progress_artifacts_written == 0 {
            return true;
        }
        let page_delta = visited_pages.saturating_sub(self.last_progress_emit_page);
        let page_budget_hit = page_delta >= options.progress_artifact_every.max(1);
        let time_budget_hit = self
            .last_progress_emit_at
            .map(|instant| {
                instant.elapsed().as_millis() as u64 >= options.progress_artifact_min_interval_ms
            })
            .unwrap_or(false);
        (page_budget_hit || time_budget_hit) && page_delta > 0
    }

    fn should_emit_partial_audit(
        &self,
        visited_pages: usize,
        options: &RuntimeAuditOptions<'_>,
    ) -> bool {
        if visited_pages == 0 {
            return false;
        }
        if self.partial_artifacts_written == 0 {
            return true;
        }
        let page_delta = visited_pages.saturating_sub(self.last_partial_emit_page);
        let page_budget_hit = page_delta >= options.partial_audit_every.max(1);
        let time_budget_hit = self
            .last_partial_emit_at
            .map(|instant| {
                instant.elapsed().as_millis() as u64 >= options.partial_audit_min_interval_ms
            })
            .unwrap_or(false);
        (page_budget_hit || time_budget_hit) && page_delta > 0
    }

    fn record_progress_artifact(&mut self, visited_pages: usize) {
        self.progress_artifacts_written += 1;
        self.last_progress_emit_page = visited_pages;
        self.last_progress_emit_at = Some(Instant::now());
    }

    fn record_partial_audit(&mut self, visited_pages: usize, duration_us: u64) {
        self.total_partial_audit_us = self.total_partial_audit_us.saturating_add(duration_us);
        self.partial_audits_built += 1;
        self.partial_artifacts_written += 1;
        self.last_partial_emit_page = visited_pages;
        self.last_partial_emit_at = Some(Instant::now());
    }

    fn record_partial_audit_build(&mut self, duration_us: u64) {
        self.total_partial_audit_build_us = self
            .total_partial_audit_build_us
            .saturating_add(duration_us);
    }

    fn record_partial_artifact_write(&mut self, duration_us: u64) {
        self.total_partial_artifact_write_us = self
            .total_partial_artifact_write_us
            .saturating_add(duration_us);
    }

    fn apply_to(&self, crawl_stats: &mut CrawlStats) {
        let elapsed_us = self
            .started_at
            .map(|instant| instant.elapsed().as_micros() as u64)
            .unwrap_or(0);
        let tracked_us = self.total_fetch_us
            + self.total_queue_selection_us
            + self.total_snapshot_write_us
            + self.total_planner_update_us
            + self.total_link_extraction_us
            + self.total_progress_callback_us
            + self.total_checkpoint_write_us
            + self.total_progress_artifact_write_us
            + self.total_sitemap_seed_us
            + self.total_optional_artifact_fetch_us
            + self.total_snapshot_build_us
            + self.total_partial_audit_build_us
            + self.total_partial_artifact_write_us
            + self.total_final_audit_us
            + self.total_rule_evaluation_us
            + self.total_policy_apply_us;
        let elapsed_ms = elapsed_us / 1_000;
        crawl_stats.elapsed_us = elapsed_us;
        crawl_stats.elapsed_ms = elapsed_ms;
        crawl_stats.pages_per_minute = if elapsed_ms == 0 {
            0
        } else {
            (((crawl_stats.visited_pages as u128) * 60_000) / (elapsed_ms as u128)) as usize
        };
        crawl_stats.checkpoints_written = self.checkpoints_written;
        crawl_stats.progress_artifacts_written = self.progress_artifacts_written;
        crawl_stats.partial_artifacts_written = self.partial_artifacts_written;
        crawl_stats.total_fetch_us = self.total_fetch_us;
        crawl_stats.total_fetch_ms = self.total_fetch_us / 1_000;
        crawl_stats.average_fetch_us = if crawl_stats.visited_pages == 0 {
            0
        } else {
            self.total_fetch_us / crawl_stats.visited_pages as u64
        };
        crawl_stats.average_fetch_ms = if crawl_stats.visited_pages == 0 {
            0
        } else {
            (self.total_fetch_us / crawl_stats.visited_pages as u64) / 1_000
        };
        crawl_stats.total_page_process_us = self.total_page_process_us;
        crawl_stats.total_page_process_ms = self.total_page_process_us / 1_000;
        crawl_stats.average_page_process_us = if crawl_stats.visited_pages == 0 {
            0
        } else {
            self.total_page_process_us / crawl_stats.visited_pages as u64
        };
        crawl_stats.average_page_process_ms = if crawl_stats.visited_pages == 0 {
            0
        } else {
            (self.total_page_process_us / crawl_stats.visited_pages as u64) / 1_000
        };
        crawl_stats.total_partial_audit_us = self.total_partial_audit_us;
        crawl_stats.total_partial_audit_ms = self.total_partial_audit_us / 1_000;
        crawl_stats.average_partial_audit_us = if self.partial_audits_built == 0 {
            0
        } else {
            self.total_partial_audit_us / self.partial_audits_built as u64
        };
        crawl_stats.average_partial_audit_ms = if self.partial_audits_built == 0 {
            0
        } else {
            (self.total_partial_audit_us / self.partial_audits_built as u64) / 1_000
        };
        crawl_stats.total_optional_artifact_fetch_us = self.total_optional_artifact_fetch_us;
        crawl_stats.total_snapshot_build_us = self.total_snapshot_build_us;
        crawl_stats.total_snapshot_write_us = self.total_snapshot_write_us;
        crawl_stats.total_queue_selection_us = self.total_queue_selection_us;
        crawl_stats.total_planner_update_us = self.total_planner_update_us;
        crawl_stats.total_link_extraction_us = self.total_link_extraction_us;
        crawl_stats.total_progress_callback_us = self.total_progress_callback_us;
        crawl_stats.total_checkpoint_write_us = self.total_checkpoint_write_us;
        crawl_stats.total_progress_artifact_write_us = self.total_progress_artifact_write_us;
        crawl_stats.total_sitemap_seed_us = self.total_sitemap_seed_us;
        crawl_stats.total_partial_audit_build_us = self.total_partial_audit_build_us;
        crawl_stats.total_partial_artifact_write_us = self.total_partial_artifact_write_us;
        crawl_stats.total_rule_evaluation_us = self.total_rule_evaluation_us;
        crawl_stats.total_policy_apply_us = self.total_policy_apply_us;
        crawl_stats.total_final_audit_us = self.total_final_audit_us;
        crawl_stats.total_overhead_us = elapsed_us.saturating_sub(tracked_us);
        crawl_stats.slowest_paths = self.slowest_paths.clone();
    }

    fn phase_timings(&self, crawl_stats: &CrawlStats) -> Vec<PhaseTiming> {
        let mut phases = vec![
            PhaseTiming {
                name: "fetch".to_string(),
                elapsed_us: self.total_fetch_us,
            },
            PhaseTiming {
                name: "queue_selection".to_string(),
                elapsed_us: self.total_queue_selection_us,
            },
            PhaseTiming {
                name: "snapshot_write".to_string(),
                elapsed_us: self.total_snapshot_write_us,
            },
            PhaseTiming {
                name: "planner_update".to_string(),
                elapsed_us: self.total_planner_update_us,
            },
            PhaseTiming {
                name: "link_extraction".to_string(),
                elapsed_us: self.total_link_extraction_us,
            },
            PhaseTiming {
                name: "progress_callback".to_string(),
                elapsed_us: self.total_progress_callback_us,
            },
            PhaseTiming {
                name: "checkpoint_write".to_string(),
                elapsed_us: self.total_checkpoint_write_us,
            },
            PhaseTiming {
                name: "progress_artifact_write".to_string(),
                elapsed_us: self.total_progress_artifact_write_us,
            },
            PhaseTiming {
                name: "sitemap_seed".to_string(),
                elapsed_us: self.total_sitemap_seed_us,
            },
            PhaseTiming {
                name: "optional_artifact_fetch".to_string(),
                elapsed_us: self.total_optional_artifact_fetch_us,
            },
            PhaseTiming {
                name: "snapshot_build".to_string(),
                elapsed_us: self.total_snapshot_build_us,
            },
            PhaseTiming {
                name: "partial_audit_build".to_string(),
                elapsed_us: self.total_partial_audit_build_us,
            },
            PhaseTiming {
                name: "partial_artifact_write".to_string(),
                elapsed_us: self.total_partial_artifact_write_us,
            },
            PhaseTiming {
                name: "final_audit".to_string(),
                elapsed_us: self.total_final_audit_us,
            },
            PhaseTiming {
                name: "rule_evaluation".to_string(),
                elapsed_us: self.total_rule_evaluation_us,
            },
            PhaseTiming {
                name: "policy_apply".to_string(),
                elapsed_us: self.total_policy_apply_us,
            },
            PhaseTiming {
                name: "overhead".to_string(),
                elapsed_us: crawl_stats.total_overhead_us,
            },
            PhaseTiming {
                name: "page_process_total".to_string(),
                elapsed_us: self.total_page_process_us,
            },
            PhaseTiming {
                name: "partial_audit_total".to_string(),
                elapsed_us: self.total_partial_audit_us,
            },
        ];
        phases.retain(|phase| phase.elapsed_us > 0);
        phases.sort_by(|left, right| right.elapsed_us.cmp(&left.elapsed_us));
        phases
    }
}

type RuntimeMaterialization = (
    Site,
    Vec<Finding>,
    CrawlStats,
    Option<String>,
    RuntimePerformance,
);

fn resolve_runtime_engine(engine: &str) -> Result<&'static str> {
    match engine {
        "auto" => Ok(if playwright_is_available() {
            "playwright"
        } else {
            "http"
        }),
        "http" => Ok("http"),
        "playwright" => {
            let doctor = probe_playwright_runtime();
            if doctor.available {
                Ok("playwright")
            } else {
                anyhow::bail!(
                    "runtime engine 'playwright' requires a local Playwright runtime; {}",
                    doctor.message
                )
            }
        }
        other => anyhow::bail!("unknown runtime engine '{other}'"),
    }
}

fn emit_progress(options: &mut RuntimeAuditOptions<'_>, event: RuntimeProgressEvent) {
    if let RuntimeProgressMode::Callback(callback) = &mut options.progress {
        callback(event);
    }
}

fn emit_progress_artifact(
    options: &mut RuntimeAuditOptions<'_>,
    artifact: &AuditArtifact,
) -> Result<()> {
    if let RuntimeArtifactMode::Callback(callback) = &mut options.progress_artifact {
        callback(artifact)?;
    }
    Ok(())
}

fn emit_partial_audit_artifact(
    options: &mut RuntimeAuditOptions<'_>,
    artifact: &AuditArtifact,
) -> Result<()> {
    if let RuntimeArtifactMode::Callback(callback) = &mut options.partial_audit_artifact {
        callback(artifact)?;
    }
    Ok(())
}

fn build_progress_runtime_artifact(
    command: &str,
    planner: &CrawlPlanner,
    crawl_findings: &[Finding],
    crawl_stats: &CrawlStats,
    performance: &RuntimePerformance,
) -> AuditArtifact {
    let status = if planner.queued_count() > 0 || planner.truncated() {
        AuditStatus::Partial
    } else {
        AuditStatus::Complete
    };
    let truncation_reason = if planner.truncated() {
        Some(format!(
            "crawl stopped at max_pages={} after visiting {} pages while at least {} routes were discovered",
            crawl_stats.max_pages,
            planner.visited_count(),
            planner.discovered_route_count()
        ))
    } else if planner.queued_count() > 0 {
        Some(format!(
            "checkpoint after visiting {} pages with {} routes still queued",
            planner.visited_count(),
            planner.queued_count()
        ))
    } else {
        None
    };
    let mut artifact = build_audit_artifact(
        command,
        crawl_findings,
        status,
        Some(crawl_stats.clone()),
        truncation_reason,
    );
    artifact.performance = Some(AuditPerformance {
        elapsed_us: crawl_stats.elapsed_us,
        phases: performance.phase_timings(crawl_stats),
        rule_groups: Vec::new(),
    });
    artifact
}

fn build_partial_runtime_artifact(
    command: &str,
    snapshot: &RuntimeSnapshotBuilder,
    planner: &CrawlPlanner,
    crawl_findings: &[Finding],
    crawl_stats: &CrawlStats,
    config: &Config,
    performance: &RuntimePerformance,
) -> Result<AuditArtifact> {
    let snapshot_started_at = Instant::now();
    let (site, snapshot_findings) = snapshot.preview(
        planner.visited_count(),
        crawl_stats.max_pages,
        planner.discovered_route_count(),
        planner.truncated(),
    )?;
    let snapshot_build_us = snapshot_started_at.elapsed().as_micros() as u64;
    let mut findings = crawl_findings.to_vec();
    findings.extend(snapshot_findings);
    let profiled = run_checks_for_site_profiled(&site, config);
    findings.extend(profiled.findings.clone());
    let mut artifact = build_audit_artifact(
        command,
        &findings,
        AuditStatus::Partial,
        Some(crawl_stats.clone()),
        Some(format!(
            "checkpoint after visiting {} pages with {} routes still queued",
            planner.visited_count(),
            planner.queued_count()
        )),
    );
    artifact.performance = Some(AuditPerformance {
        elapsed_us: crawl_stats.elapsed_us,
        phases: {
            let mut phases = performance.phase_timings(crawl_stats);
            phases.push(PhaseTiming {
                name: "partial_snapshot_build".to_string(),
                elapsed_us: snapshot_build_us,
            });
            phases.sort_by(|left, right| right.elapsed_us.cmp(&left.elapsed_us));
            phases
        },
        rule_groups: profiled.rule_timings,
    });
    Ok(artifact)
}

fn checkpoint_state(
    checkpoint_path: &Path,
    base_url: &str,
    engine: &str,
    planner: &CrawlPlanner,
    snapshot: &RuntimeSnapshotBuilder,
    crawl_findings: &[Finding],
    crawl_stats: &CrawlStats,
) -> Result<()> {
    let checkpoint = RuntimeCheckpoint {
        version: 1,
        base_url: base_url.to_string(),
        engine: engine.to_string(),
        planner: planner.checkpoint_state(),
        snapshot: snapshot.checkpoint_state(),
        crawl_findings: crawl_findings.to_vec(),
        crawl_stats: crawl_stats.clone(),
    };
    fs::write(checkpoint_path, serde_json::to_string_pretty(&checkpoint)?)?;
    Ok(())
}

fn load_checkpoint(checkpoint_path: &Path, max_pages: usize) -> Result<RuntimeCheckpoint> {
    let text = fs::read_to_string(checkpoint_path)?;
    let mut checkpoint = serde_json::from_str::<RuntimeCheckpoint>(&text)?;
    checkpoint.planner.max_pages = max_pages;
    checkpoint.crawl_stats.max_pages = max_pages;
    Ok(checkpoint)
}

fn materialize_runtime_site(
    base_url: &str,
    max_pages: usize,
    engine: &str,
    config: &Config,
    options: &mut RuntimeAuditOptions<'_>,
) -> Result<RuntimeMaterialization> {
    let runtime = config.runtime();
    let (mut planner, mut snapshot, mut crawl_findings, mut crawl_stats, mut performance) =
        if let Some(resume_from) = options.resume_from {
            let checkpoint = load_checkpoint(resume_from, max_pages)?;
            let resumed_elapsed_us = checkpoint.crawl_stats.elapsed_us;
            (
                CrawlPlanner::from_checkpoint(checkpoint.planner, max_pages),
                RuntimeSnapshotBuilder::from_state(checkpoint.snapshot),
                checkpoint.crawl_findings,
                checkpoint.crawl_stats,
                RuntimePerformance::from_started_elapsed(resumed_elapsed_us),
            )
        } else {
            (
                CrawlPlanner::new(base_url, max_pages),
                RuntimeSnapshotBuilder::new(),
                Vec::new(),
                CrawlStats {
                    engine: engine.to_string(),
                    max_pages,
                    ..CrawlStats::default()
                },
                RuntimePerformance::new(),
            )
        };

    if options.resume_from.is_none() {
        for seed in runtime.crawl_seeds {
            planner.seed_from_user_input(seed, &runtime);
        }

        if runtime.crawl_use_sitemap {
            let mut visited_sitemaps = BTreeSet::new();
            let sitemap_base = planner.normalized_base().to_string();
            let sitemap_seed_started_at = Instant::now();
            for sitemap_name in ["sitemap.xml", "sitemap-index.xml", "sitemap_index.xml"] {
                let _ = seed_routes_from_sitemap(
                    &mut planner,
                    &format!("{}{}", sitemap_base, sitemap_name),
                    &runtime,
                    &mut visited_sitemaps,
                );
            }
            performance.record_sitemap_seed(sitemap_seed_started_at.elapsed().as_micros() as u64);
        }
    }

    let mut fetcher = RuntimeFetcher::new(engine, &runtime)?;
    let effective_workers = if engine == "http" {
        runtime.max_workers.max(1)
    } else {
        1
    };
    loop {
        let queue_selection_started_at = Instant::now();
        let mut batch = Vec::new();
        while batch.len() < effective_workers {
            let Some(next_url) = planner.next_url(&runtime) else {
                break;
            };
            batch.push(next_url);
        }
        performance.record_queue_selection(queue_selection_started_at.elapsed().as_micros() as u64);
        if batch.is_empty() {
            break;
        }

        let outcomes = fetcher.fetch_batch(&batch, options.fetch_retry_budget);
        for outcome in outcomes {
            crawl_stats.fetch_retries += outcome.retries;
            performance.record_fetch(outcome.elapsed_us);
            let FetchOutcome {
                url: current,
                result,
                retries: _,
                elapsed_us: fetch_elapsed_us,
            } = outcome;
            let fetched = match result {
                Ok(fetched) => fetched,
                Err(error) => {
                    let route = route_from_urlish(&current).unwrap_or_default();
                    crawl_stats.fetch_failures += 1;
                    performance.apply_to(&mut crawl_stats);
                    crawl_findings.push(Finding {
                        rule_id: "CRW001".to_string(),
                        message: format!("failed to fetch URL: {} ({})", current, error),
                        path: response_report_path(&route),
                        line: 1,
                        column: 1,
                        severity: "error".to_string(),
                        suggestion: None,
                        scope: FindingScope::Page,
                    });
                    continue;
                }
            };
            let Some(body) = fetched.body else {
                let route = route_from_urlish(&current).unwrap_or_default();
                crawl_stats.fetch_failures += 1;
                performance.apply_to(&mut crawl_stats);
                crawl_findings.push(Finding {
                    rule_id: "CRW001".to_string(),
                    message: format!("failed to fetch URL: {}", current),
                    path: response_report_path(&route),
                    line: 1,
                    column: 1,
                    severity: "error".to_string(),
                    suggestion: None,
                    scope: FindingScope::Page,
                });
                continue;
            };
            if !is_html_content_type(fetched.content_type.as_deref()) {
                crawl_stats.skipped_non_html += 1;
                performance.apply_to(&mut crawl_stats);
                continue;
            }
            let effective_host = host_for_url(&fetched.effective_url);
            if !same_site_host(&effective_host, planner.base_host()) {
                performance.apply_to(&mut crawl_stats);
                continue;
            }
            let process_started_at = Instant::now();
            let planner_started_at = Instant::now();
            planner.align_with_effective_url(&fetched.effective_url);
            let route = route_from_urlish(&fetched.effective_url).unwrap_or_default();
            let snapshot_write_started_at = Instant::now();
            snapshot.write_page(&route, &body, &fetched.headers)?;
            performance
                .record_snapshot_write(snapshot_write_started_at.elapsed().as_micros() as u64);

            let extraction_started_at = Instant::now();
            for target in extract_internal_links(&body, planner.base_host()) {
                if !should_enqueue_link(&target) {
                    continue;
                }
                planner.discover_link_target(&target, &runtime);
            }
            performance.record_link_extraction(extraction_started_at.elapsed().as_micros() as u64);
            performance.record_planner_update(planner_started_at.elapsed().as_micros() as u64);

            crawl_stats.visited_pages = planner.visited_count();
            crawl_stats.discovered_internal_routes = planner.discovered_route_count();
            crawl_stats.queued_routes_remaining = planner.queued_count();
            crawl_stats.truncated = planner.truncated();
            let page_process_us = process_started_at.elapsed().as_micros() as u64;
            performance.record_page_process(
                &fetched.effective_url,
                fetch_elapsed_us,
                page_process_us,
            );
            performance.apply_to(&mut crawl_stats);
            let progress_started_at = Instant::now();
            emit_progress(
                options,
                RuntimeProgressEvent {
                    phase: "progress".to_string(),
                    engine: engine.to_string(),
                    current_url: Some(fetched.effective_url),
                    visited_pages: crawl_stats.visited_pages,
                    discovered_internal_routes: crawl_stats.discovered_internal_routes,
                    queued_routes_remaining: crawl_stats.queued_routes_remaining,
                    fetch_failures: crawl_stats.fetch_failures,
                    fetch_retries: crawl_stats.fetch_retries,
                    skipped_non_html: crawl_stats.skipped_non_html,
                    truncated: crawl_stats.truncated,
                    elapsed_ms: crawl_stats.elapsed_ms,
                    elapsed_us: crawl_stats.elapsed_us,
                    pages_per_minute: crawl_stats.pages_per_minute,
                    average_fetch_ms: crawl_stats.average_fetch_ms,
                    average_page_process_ms: crawl_stats.average_page_process_ms,
                    average_partial_audit_ms: crawl_stats.average_partial_audit_ms,
                    checkpoints_written: crawl_stats.checkpoints_written,
                    progress_artifacts_written: crawl_stats.progress_artifacts_written,
                    partial_artifacts_written: crawl_stats.partial_artifacts_written,
                },
            );
            performance.record_progress_callback(progress_started_at.elapsed().as_micros() as u64);
        }

        if let Some(checkpoint_path) = options.checkpoint_path
            && crawl_stats.visited_pages > 0
            && crawl_stats.visited_pages % options.checkpoint_every.max(1) == 0
        {
            performance.record_checkpoint();
            performance.apply_to(&mut crawl_stats);
            let checkpoint_started_at = Instant::now();
            checkpoint_state(
                checkpoint_path,
                base_url,
                engine,
                &planner,
                &snapshot,
                &crawl_findings,
                &crawl_stats,
            )?;
            performance.record_checkpoint_write(checkpoint_started_at.elapsed().as_micros() as u64);
        }
        if performance.should_emit_progress_artifact(crawl_stats.visited_pages, options) {
            let artifact = build_progress_runtime_artifact(
                options.artifact_command,
                &planner,
                &crawl_findings,
                &crawl_stats,
                &performance,
            );
            let progress_artifact_started_at = Instant::now();
            emit_progress_artifact(options, &artifact)?;
            performance.record_progress_artifact_write(
                progress_artifact_started_at.elapsed().as_micros() as u64,
            );
            performance.record_progress_artifact(crawl_stats.visited_pages);
            performance.apply_to(&mut crawl_stats);
        }
        if performance.should_emit_partial_audit(crawl_stats.visited_pages, options) {
            let partial_build_started_at = Instant::now();
            let artifact = build_partial_runtime_artifact(
                options.artifact_command,
                &snapshot,
                &planner,
                &crawl_findings,
                &crawl_stats,
                config,
                &performance,
            )?;
            let partial_build_duration_us = partial_build_started_at.elapsed().as_micros() as u64;
            performance.record_partial_audit_build(partial_build_duration_us);
            let partial_write_started_at = Instant::now();
            emit_partial_audit_artifact(options, &artifact)?;
            let partial_write_duration_us = partial_write_started_at.elapsed().as_micros() as u64;
            performance.record_partial_artifact_write(partial_write_duration_us);
            let partial_duration_us =
                partial_build_duration_us.saturating_add(partial_write_duration_us);
            performance.record_partial_audit(crawl_stats.visited_pages, partial_duration_us);
            performance.apply_to(&mut crawl_stats);
        }
    }

    let optional_fetch_us =
        snapshot.capture_optional_artifacts(planner.normalized_base(), &runtime)?;
    performance.record_optional_artifact_fetch(optional_fetch_us);
    let snapshot_started_at = Instant::now();
    let (site, snapshot_findings) = snapshot.finalize(
        planner.visited_count(),
        max_pages,
        planner.discovered_route_count(),
        planner.truncated(),
    )?;
    performance.record_snapshot_build(snapshot_started_at.elapsed().as_micros() as u64);
    crawl_stats.visited_pages = planner.visited_count();
    crawl_stats.discovered_internal_routes = planner.discovered_route_count();
    crawl_stats.queued_routes_remaining = planner.queued_count();
    crawl_stats.truncated = planner.truncated();
    performance.apply_to(&mut crawl_stats);
    crawl_findings.extend(snapshot_findings);
    let truncation_reason = if planner.truncated() {
        Some(format!(
            "crawl stopped at max_pages={} after visiting {} pages while at least {} routes were discovered",
            max_pages,
            planner.visited_count(),
            planner.discovered_route_count()
        ))
    } else {
        None
    };
    if let Some(checkpoint_path) = options.checkpoint_path
        && site
            .crawl_meta
            .as_ref()
            .map(|meta| !meta.truncated)
            .unwrap_or(true)
    {
        let _ = fs::remove_file(checkpoint_path);
    }
    Ok((
        site,
        crawl_findings,
        crawl_stats,
        truncation_reason,
        performance,
    ))
}

fn seed_routes_from_sitemap(
    planner: &mut CrawlPlanner,
    sitemap_url: &str,
    runtime: &crate::config::RuntimeConfig<'_>,
    visited_sitemaps: &mut BTreeSet<String>,
) -> Result<()> {
    if !visited_sitemaps.insert(sitemap_url.to_string()) {
        return Ok(());
    }
    let fetched = fetch_with_http(
        sitemap_url,
        runtime.crawl_headers,
        runtime.crawl_cookies,
        runtime.crawl_basic_auth,
    )?;
    if fetched.status_code.unwrap_or(500) >= 400
        || !fetched
            .content_type
            .as_deref()
            .unwrap_or_default()
            .contains("xml")
    {
        return Ok(());
    }
    let Some(body) = fetched.body else {
        return Ok(());
    };
    for loc in read_loc_values(&body) {
        if loc.trim().ends_with(".xml") {
            seed_routes_from_sitemap(planner, &loc, runtime, visited_sitemaps)?;
        } else {
            planner.seed_from_sitemap_loc(&loc, runtime);
        }
    }
    Ok(())
}

pub fn run_runtime_audit(
    base_url: &str,
    max_pages: usize,
    engine: &str,
    config: &Config,
) -> Result<RuntimeAudit> {
    run_runtime_audit_with_options(
        base_url,
        max_pages,
        engine,
        config,
        &mut RuntimeAuditOptions::default(),
    )
}

pub fn run_runtime_audit_with_options(
    base_url: &str,
    max_pages: usize,
    engine: &str,
    config: &Config,
    options: &mut RuntimeAuditOptions<'_>,
) -> Result<RuntimeAudit> {
    let effective_engine = resolve_runtime_engine(engine)?;
    let (site, crawl_findings, mut crawl_stats, truncation_reason, mut performance) =
        materialize_runtime_site(base_url, max_pages, effective_engine, config, options)?;
    let final_started_at = Instant::now();
    let profiled = run_checks_for_site_profiled(&site, config);
    performance.record_rule_evaluation(
        profiled
            .rule_timings
            .iter()
            .map(|timing| timing.elapsed_us)
            .sum(),
    );
    performance.record_policy_apply(profiled.policy_apply_us);
    performance.record_final_audit(final_started_at.elapsed().as_micros() as u64);
    performance.apply_to(&mut crawl_stats);
    let mut findings = crawl_findings.clone();
    findings.extend(profiled.findings.clone());
    let status = if crawl_stats.truncated {
        AuditStatus::Partial
    } else {
        AuditStatus::Complete
    };
    let progress_started_at = Instant::now();
    emit_progress(
        options,
        RuntimeProgressEvent {
            phase: "complete".to_string(),
            engine: effective_engine.to_string(),
            current_url: None,
            visited_pages: crawl_stats.visited_pages,
            discovered_internal_routes: crawl_stats.discovered_internal_routes,
            queued_routes_remaining: crawl_stats.queued_routes_remaining,
            fetch_failures: crawl_stats.fetch_failures,
            fetch_retries: crawl_stats.fetch_retries,
            skipped_non_html: crawl_stats.skipped_non_html,
            truncated: crawl_stats.truncated,
            elapsed_ms: crawl_stats.elapsed_ms,
            elapsed_us: crawl_stats.elapsed_us,
            pages_per_minute: crawl_stats.pages_per_minute,
            average_fetch_ms: crawl_stats.average_fetch_ms,
            average_page_process_ms: crawl_stats.average_page_process_ms,
            average_partial_audit_ms: crawl_stats.average_partial_audit_ms,
            checkpoints_written: crawl_stats.checkpoints_written,
            progress_artifacts_written: crawl_stats.progress_artifacts_written,
            partial_artifacts_written: crawl_stats.partial_artifacts_written,
        },
    );
    performance.record_progress_callback(progress_started_at.elapsed().as_micros() as u64);
    performance.apply_to(&mut crawl_stats);
    let performance_summary = AuditPerformance {
        elapsed_us: crawl_stats.elapsed_us,
        phases: performance.phase_timings(&crawl_stats),
        rule_groups: profiled.rule_timings,
    };
    Ok(RuntimeAudit {
        site,
        crawl_findings,
        findings,
        status,
        crawl_stats,
        truncation_reason,
        performance: Some(performance_summary),
    })
}

pub fn verify_runtime_audit(audit: &RuntimeAudit, baseline_findings: &[Finding]) -> DiffResult {
    diff_finding_sets(baseline_findings, &audit.findings)
}

pub fn runtime_doctor() -> PlaywrightDoctor {
    probe_playwright_runtime()
}

#[cfg(test)]
mod tests {
    use super::{
        RuntimeArtifactMode, RuntimeAuditOptions, RuntimeProgressMode, run_runtime_audit,
        run_runtime_audit_with_options, verify_runtime_audit,
    };
    use crate::config::{Config, default_rule_switches};
    use seogeo_contracts::{AuditStatus, CrawlStats, Finding, FindingScope};
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::ErrorKind;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::thread;
    use std::time::{Duration, Instant};

    fn respond(
        mut stream: TcpStream,
        status: &str,
        content_type: &str,
        body: &str,
        extra_headers: &[(&str, &str)],
    ) {
        let mut response = format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
            status,
            content_type,
            body.len()
        );
        for (key, value) in extra_headers {
            response.push_str(&format!("{}: {}\r\n", key, value));
        }
        response.push_str("\r\n");
        response.push_str(body);
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    fn spawn_fixture_server<F>(min_requests: usize, handler: F) -> (String, thread::JoinHandle<()>)
    where
        F: Fn(TcpStream, String, SocketAddr) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut served = 0usize;
            let started = Instant::now();
            let mut last_request = Instant::now();
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let mut buffer = [0_u8; 4096];
                        let size = stream.read(&mut buffer).unwrap();
                        let request = String::from_utf8_lossy(&buffer[..size]).into_owned();
                        served += 1;
                        last_request = Instant::now();
                        handler(stream, request, address);
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if (served >= min_requests.max(1)
                            && last_request.elapsed() > Duration::from_millis(150))
                            || started.elapsed() > Duration::from_secs(30)
                        {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("server accept failed: {error}"),
                }
            }
        });
        (format!("http://{}", address), handle)
    }

    fn spawn_server(expected_requests: usize) -> (String, thread::JoinHandle<()>) {
        spawn_fixture_server(expected_requests, |stream, request, _address| {
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            match path {
                "/" => respond(
                    stream,
                    "200 OK",
                    "text/html",
                    "<html><head><title>Home</title><meta name=\"description\" content=\"Root\"><link rel=\"canonical\" href=\"http://example.test/\"></head><body><h1>Home</h1><a href=\"/about\">About</a></body></html>",
                    &[],
                ),
                "/about" => respond(
                    stream,
                    "200 OK",
                    "text/html",
                    "<html><head><meta name=\"description\" content=\"About page\"></head><body><h1>About</h1></body></html>",
                    &[("X-Robots-Tag", "noindex")],
                ),
                "/robots.txt" => respond(
                    stream,
                    "200 OK",
                    "text/plain",
                    "User-agent: *\nAllow: /\n",
                    &[],
                ),
                "/llms.txt" => respond(
                    stream,
                    "200 OK",
                    "text/plain",
                    "# Site\n\n## Pages\n- [Home](/)\n",
                    &[],
                ),
                "/sitemap.xml" => respond(
                    stream,
                    "200 OK",
                    "application/xml",
                    "<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>http://example.test/</loc></url><url><loc>http://example.test/about</loc></url></urlset>",
                    &[],
                ),
                _ => respond(stream, "404 Not Found", "text/plain", "missing", &[]),
            }
        })
    }

    fn spawn_sitemap_index_server(expected_requests: usize) -> (String, thread::JoinHandle<()>) {
        spawn_fixture_server(expected_requests, move |stream, request, address| {
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            match path {
                "/" => respond(
                    stream,
                    "200 OK",
                    "text/html",
                    "<html><head><title>Home</title><meta name=\"description\" content=\"Root\"><link rel=\"canonical\" href=\"http://example.test/\"></head><body><h1>Home</h1></body></html>",
                    &[],
                ),
                "/from-sitemap" => respond(
                    stream,
                    "200 OK",
                    "text/html",
                    "<html><head><title>Indexed</title><meta name=\"description\" content=\"Indexed\"><link rel=\"canonical\" href=\"http://example.test/from-sitemap\"></head><body><h1>Indexed</h1></body></html>",
                    &[],
                ),
                "/robots.txt" => respond(
                    stream,
                    "200 OK",
                    "text/plain",
                    "User-agent: *\nAllow: /\n",
                    &[],
                ),
                "/llms.txt" => respond(stream, "404 Not Found", "text/plain", "missing", &[]),
                "/sitemap.xml" => {
                    respond(stream, "404 Not Found", "application/xml", "missing", &[])
                }
                "/sitemap-index.xml" => respond(
                    stream,
                    "200 OK",
                    "application/xml",
                    &format!(
                        "<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><sitemap><loc>http://{}/nested-sitemap.xml</loc></sitemap></sitemapindex>",
                        address
                    ),
                    &[],
                ),
                "/nested-sitemap.xml" => respond(
                    stream,
                    "200 OK",
                    "application/xml",
                    &format!(
                        "<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>http://{}/from-sitemap</loc></url></urlset>",
                        address
                    ),
                    &[],
                ),
                "/sitemap_index.xml" => {
                    respond(stream, "404 Not Found", "application/xml", "missing", &[])
                }
                _ => respond(stream, "404 Not Found", "text/plain", "missing", &[]),
            }
        })
    }

    fn html_only_config() -> Config {
        let mut config = Config {
            checks: default_rule_switches()
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
            ..Config::default()
        };
        for key in [
            "links",
            "sitemap",
            "robots",
            "social",
            "schema",
            "llm",
            "content",
            "structure",
        ] {
            config.checks.insert(key.to_string(), false);
        }
        config
    }

    #[test]
    fn runtime_audit_crawls_http_site() {
        let (base_url, handle) = spawn_server(6);
        let audit = run_runtime_audit(&base_url, 10, "http", &html_only_config()).unwrap();
        assert!(audit.site.has_route(""));
        assert!(audit.site.has_route("about"));
        assert!(
            audit
                .findings
                .iter()
                .any(|finding| finding.rule_id == "SEO001" || finding.rule_id == "SEO004")
        );
        handle.join().unwrap();
    }

    #[test]
    fn runtime_audit_accepts_html_charset_content_types() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for _ in 0..8 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0_u8; 4096];
                let size = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..size]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                match path {
                    "/" => respond(
                        stream,
                        "200 OK",
                        "text/html; charset=utf-8",
                        "<html><head><meta name=\"description\" content=\"Root\"><link rel=\"canonical\" href=\"http://example.test/\"></head><body><h1>Home</h1><a href=\"/about\">About</a></body></html>",
                        &[],
                    ),
                    "/about" => respond(
                        stream,
                        "200 OK",
                        "text/html; charset=utf-8",
                        "<html><head><meta name=\"description\" content=\"About page\"></head><body><h1>About</h1></body></html>",
                        &[],
                    ),
                    "/robots.txt" => respond(
                        stream,
                        "200 OK",
                        "text/plain; charset=utf-8",
                        "User-agent: *\nAllow: /\n",
                        &[],
                    ),
                    "/llms.txt" => respond(
                        stream,
                        "404 Not Found",
                        "text/plain; charset=utf-8",
                        "missing",
                        &[],
                    ),
                    _ => respond(stream, "404 Not Found", "text/plain", "missing", &[]),
                }
            }
        });

        let audit = run_runtime_audit(
            &format!("http://{}", address),
            10,
            "http",
            &html_only_config(),
        )
        .unwrap();
        assert!(audit.site.has_route(""));
        assert!(audit.site.has_route("about"));
        assert!(
            audit
                .findings
                .iter()
                .any(|finding| finding.rule_id == "SEO001" || finding.rule_id == "SEO004")
        );
        handle.join().unwrap();
    }

    #[test]
    fn runtime_audit_seeds_from_sitemap_indexes() {
        let (base_url, handle) = spawn_sitemap_index_server(9);
        let audit = run_runtime_audit(&base_url, 10, "http", &html_only_config()).unwrap();
        assert!(audit.site.has_route(""));
        assert!(audit.site.has_route("from-sitemap"));
        handle.join().unwrap();
    }

    #[test]
    fn runtime_verify_reports_regressions() {
        let audit = super::RuntimeAudit {
            site: crate::site::Site {
                root: PathBuf::new(),
                pages: Vec::new(),
                route_page_indices: BTreeMap::new(),
                indexed_paths: BTreeSet::new(),
                inbound_links: BTreeMap::new(),
                llms_text: None,
                robots_text: None,
                sitemap_routes: BTreeSet::new(),
                sitemap_error: None,
                deployment_model: crate::site::DeploymentModel::RuntimeSnapshot,
                deployment_markers: Vec::new(),
                crawl_meta: None,
            },
            crawl_findings: Vec::new(),
            findings: vec![Finding {
                rule_id: "SEO001".to_string(),
                message: "missing <title>".to_string(),
                path: "crawl/about/index.html".to_string(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
                scope: FindingScope::Page,
            }],
            status: AuditStatus::Complete,
            crawl_stats: CrawlStats {
                engine: "http".to_string(),
                max_pages: 10,
                ..CrawlStats::default()
            },
            truncation_reason: None,
            performance: None,
        };
        let diff = verify_runtime_audit(&audit, &[]);
        assert_eq!(diff.new_findings.len(), 1);
    }

    #[test]
    fn runtime_audit_reports_truncated_crawl_coverage() {
        let (base_url, handle) = spawn_server(1);
        let mut config = html_only_config();
        config.checks.insert("links".to_string(), true);
        let audit = run_runtime_audit(&base_url, 1, "http", &config).unwrap();
        assert!(
            audit
                .findings
                .iter()
                .any(|finding| finding.rule_id == "CRW003")
        );
        assert!(
            !audit
                .findings
                .iter()
                .any(|finding| finding.rule_id == "LNK001")
        );
        handle.join().unwrap();
    }

    #[test]
    fn runtime_audit_emits_partial_artifacts_during_checkpoint_flushes() {
        let (base_url, handle) = spawn_server(6);
        let mut partial_artifacts = Vec::new();
        let mut artifact_callback =
            |artifact: &seogeo_contracts::AuditArtifact| -> anyhow::Result<()> {
                partial_artifacts.push(artifact.clone());
                Ok(())
            };
        let mut options = RuntimeAuditOptions {
            checkpoint_every: 1,
            partial_audit_every: 100,
            progress: RuntimeProgressMode::Off,
            artifact_command: "crawl",
            progress_artifact: RuntimeArtifactMode::Off,
            partial_audit_artifact: RuntimeArtifactMode::Callback(&mut artifact_callback),
            ..RuntimeAuditOptions::default()
        };

        let audit =
            run_runtime_audit_with_options(&base_url, 1, "http", &html_only_config(), &mut options)
                .unwrap();

        assert_eq!(audit.status, AuditStatus::Partial);
        assert!(!partial_artifacts.is_empty());
        assert!(
            partial_artifacts
                .iter()
                .all(|artifact| artifact.status == AuditStatus::Partial)
        );
        assert!(partial_artifacts.iter().all(|artifact| {
            artifact
                .findings
                .iter()
                .any(|finding| finding.rule_id == "CRW003")
        }));
        assert!(audit.crawl_stats.partial_artifacts_written >= 1);
        assert!(audit.crawl_stats.average_fetch_ms > 0);
        handle.join().unwrap();
    }

    #[test]
    fn runtime_audit_handles_playwright_according_to_local_runtime() {
        if super::playwright_is_available() {
            let (base_url, handle) = spawn_server(8);
            let audit = run_runtime_audit(&base_url, 1, "playwright", &html_only_config())
                .expect("playwright should run when the local runtime is installed");
            assert!(
                audit
                    .findings
                    .iter()
                    .any(|finding| finding.path == "crawl/index.html")
            );
            handle.join().unwrap();
        } else {
            let error =
                run_runtime_audit("https://example.com", 10, "playwright", &Config::default())
                    .expect_err("playwright without a runner should fail");
            assert!(
                error
                    .to_string()
                    .contains("requires a local Playwright runtime")
            );
        }
    }

    #[test]
    fn runtime_audit_reuses_playwright_session_across_multiple_pages() {
        if !super::playwright_is_available() {
            return;
        }
        let (base_url, handle) = spawn_server(8);
        let audit = run_runtime_audit(&base_url, 2, "playwright", &html_only_config()).unwrap();
        assert!(audit.site.has_route(""));
        assert!(audit.site.has_route("about"));
        assert!(
            !audit
                .findings
                .iter()
                .any(|finding| finding.rule_id == "CRW001")
        );
        handle.join().unwrap();
    }

    #[test]
    fn runtime_audit_rejects_unknown_engine() {
        let error = run_runtime_audit("https://example.com", 10, "invalid", &Config::default())
            .expect_err("invalid engines should fail");
        assert!(error.to_string().contains("unknown runtime engine"));
    }
}
