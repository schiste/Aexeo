mod fetcher;
mod graph;
mod http;
mod planner;
mod playwright;
mod snapshot;

use aexeo_contracts::{
    AuditArtifact, AuditPerformance, AuditStatus, CrawlStats, Finding, FindingScope,
    PerformanceBottleneck, PerformanceBudget, PerformanceBudgetReport, PerformanceBudgetViolation,
    PerformanceDiffReport, PerformanceDiffSummary, PerformanceDiffThresholds,
    PerformanceMetricDelta, PhaseTiming, RuleTiming, SlowCrawlPath,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
    samples: BTreeMap<String, Vec<u64>>,
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
        self.record_sample("fetch", duration_us);
    }

    fn record_page_process(&mut self, url: &str, fetch_us: u64, process_us: u64) {
        self.total_page_process_us = self.total_page_process_us.saturating_add(process_us);
        self.record_sample("page_process_total", process_us);
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
        self.record_sample("queue_selection", duration_us);
    }

    fn record_snapshot_write(&mut self, duration_us: u64) {
        self.total_snapshot_write_us = self.total_snapshot_write_us.saturating_add(duration_us);
        self.record_sample("snapshot_write", duration_us);
    }

    fn record_planner_update(&mut self, duration_us: u64) {
        self.total_planner_update_us = self.total_planner_update_us.saturating_add(duration_us);
        self.record_sample("planner_update", duration_us);
    }

    fn record_link_extraction(&mut self, duration_us: u64) {
        self.total_link_extraction_us = self.total_link_extraction_us.saturating_add(duration_us);
        self.record_sample("link_extraction", duration_us);
    }

    fn record_progress_callback(&mut self, duration_us: u64) {
        self.total_progress_callback_us =
            self.total_progress_callback_us.saturating_add(duration_us);
        self.record_sample("progress_callback", duration_us);
    }

    fn record_checkpoint_write(&mut self, duration_us: u64) {
        self.total_checkpoint_write_us = self.total_checkpoint_write_us.saturating_add(duration_us);
        self.record_sample("checkpoint_write", duration_us);
    }

    fn record_progress_artifact_write(&mut self, duration_us: u64) {
        self.total_progress_artifact_write_us = self
            .total_progress_artifact_write_us
            .saturating_add(duration_us);
        self.record_sample("progress_artifact_write", duration_us);
    }

    fn record_optional_artifact_fetch(&mut self, duration_us: u64) {
        self.total_optional_artifact_fetch_us = self
            .total_optional_artifact_fetch_us
            .saturating_add(duration_us);
        self.record_sample("optional_artifact_fetch", duration_us);
    }

    fn record_sitemap_seed(&mut self, duration_us: u64) {
        self.total_sitemap_seed_us = self.total_sitemap_seed_us.saturating_add(duration_us);
        self.record_sample("sitemap_seed", duration_us);
    }

    fn record_snapshot_build(&mut self, duration_us: u64) {
        self.total_snapshot_build_us = self.total_snapshot_build_us.saturating_add(duration_us);
        self.record_sample("snapshot_build", duration_us);
    }

    fn record_final_audit(&mut self, duration_us: u64) {
        self.total_final_audit_us = self.total_final_audit_us.saturating_add(duration_us);
        self.record_sample("final_audit", duration_us);
    }

    fn record_rule_evaluation(&mut self, duration_us: u64) {
        self.total_rule_evaluation_us = self.total_rule_evaluation_us.saturating_add(duration_us);
        self.record_sample("rule_evaluation", duration_us);
    }

    fn record_policy_apply(&mut self, duration_us: u64) {
        self.total_policy_apply_us = self.total_policy_apply_us.saturating_add(duration_us);
        self.record_sample("policy_apply", duration_us);
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
        self.record_sample("partial_audit_total", duration_us);
        self.partial_audits_built += 1;
        self.partial_artifacts_written += 1;
        self.last_partial_emit_page = visited_pages;
        self.last_partial_emit_at = Some(Instant::now());
    }

    fn record_partial_audit_build(&mut self, duration_us: u64) {
        self.total_partial_audit_build_us = self
            .total_partial_audit_build_us
            .saturating_add(duration_us);
        self.record_sample("partial_audit_build", duration_us);
    }

    fn record_partial_artifact_write(&mut self, duration_us: u64) {
        self.total_partial_artifact_write_us = self
            .total_partial_artifact_write_us
            .saturating_add(duration_us);
        self.record_sample("partial_artifact_write", duration_us);
    }

    fn record_sample(&mut self, name: &str, duration_us: u64) {
        self.samples
            .entry(name.to_string())
            .or_default()
            .push(duration_us);
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
            self.phase("fetch", self.total_fetch_us, "cumulative"),
            self.phase(
                "queue_selection",
                self.total_queue_selection_us,
                "cumulative",
            ),
            self.phase("snapshot_write", self.total_snapshot_write_us, "cumulative"),
            self.phase("planner_update", self.total_planner_update_us, "cumulative"),
            self.phase(
                "link_extraction",
                self.total_link_extraction_us,
                "cumulative",
            ),
            self.phase(
                "progress_callback",
                self.total_progress_callback_us,
                "cumulative",
            ),
            self.phase(
                "checkpoint_write",
                self.total_checkpoint_write_us,
                "cumulative",
            ),
            self.phase(
                "progress_artifact_write",
                self.total_progress_artifact_write_us,
                "cumulative",
            ),
            self.phase("sitemap_seed", self.total_sitemap_seed_us, "cumulative"),
            self.phase(
                "optional_artifact_fetch",
                self.total_optional_artifact_fetch_us,
                "cumulative",
            ),
            self.phase("snapshot_build", self.total_snapshot_build_us, "cumulative"),
            self.phase(
                "partial_audit_build",
                self.total_partial_audit_build_us,
                "cumulative",
            ),
            self.phase(
                "partial_artifact_write",
                self.total_partial_artifact_write_us,
                "cumulative",
            ),
            self.phase("final_audit", self.total_final_audit_us, "cumulative"),
            self.phase("rule_evaluation", self.total_rule_evaluation_us, "nested"),
            self.phase("policy_apply", self.total_policy_apply_us, "nested"),
            self.phase("overhead", crawl_stats.total_overhead_us, "cumulative"),
            self.phase("page_process_total", self.total_page_process_us, "derived"),
            self.phase(
                "partial_audit_total",
                self.total_partial_audit_us,
                "derived",
            ),
        ];
        phases.retain(|phase| phase.elapsed_us > 0);
        phases.sort_by(|left, right| right.elapsed_us.cmp(&left.elapsed_us));
        phases
    }

    fn phase(&self, name: &str, elapsed_us: u64, basis: &str) -> PhaseTiming {
        let samples = self.samples.get(name).map(Vec::as_slice).unwrap_or(&[]);
        let mut phase = PhaseTiming {
            name: name.to_string(),
            elapsed_us,
            basis: basis.to_string(),
            ..PhaseTiming::default()
        };
        apply_sample_distribution(&mut phase, samples);
        phase
    }
}

fn percentile(sorted: &[u64], percentile_basis_points: u64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let max_index = sorted.len() - 1;
    let index = ((max_index as u64) * percentile_basis_points).div_ceil(10_000) as usize;
    sorted[index.min(max_index)]
}

fn apply_sample_distribution(phase: &mut PhaseTiming, samples: &[u64]) {
    if samples.is_empty() {
        return;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    phase.sample_count = sorted.len();
    phase.min_us = *sorted.first().unwrap_or(&0);
    phase.max_us = *sorted.last().unwrap_or(&0);
    phase.p50_us = percentile(&sorted, 5_000);
    phase.p75_us = percentile(&sorted, 7_500);
    phase.p95_us = percentile(&sorted, 9_500);
    phase.p99_us = percentile(&sorted, 9_900);
}

fn capped_share_basis_points(elapsed_us: u64, total_us: u64) -> u32 {
    if total_us == 0 {
        return 0;
    }
    ((elapsed_us.saturating_mul(10_000)) / total_us).min(10_000) as u32
}

fn uncapped_share_basis_points(elapsed_us: u64, total_us: u64) -> u32 {
    if total_us == 0 {
        return 0;
    }
    ((elapsed_us as u128).saturating_mul(10_000) / total_us as u128).min(u32::MAX as u128) as u32
}

fn format_share(share_basis_points: u32) -> String {
    format!(
        "{}.{:02}%",
        share_basis_points / 100,
        share_basis_points % 100
    )
}

fn phase_recommendation(name: &str) -> Option<String> {
    match name {
        "fetch" => Some(
            "optimize network concurrency, caching, robots/sitemap seeds, and avoid Playwright unless rendered DOM is required"
                .to_string(),
        ),
        "page_process_total" | "link_extraction" | "planner_update" => Some(
            "profile extraction and graph-update work; large HTML or dense internal-link graphs may need batching"
                .to_string(),
        ),
        "rule_evaluation" | "final_audit" => Some(
            "inspect slowest rule groups and disable or policy-gate expensive checks for exploratory crawls"
                .to_string(),
        ),
        "partial_audit_total" | "partial_audit_build" => Some(
            "increase partial-audit intervals for large crawls when progress artifacts are not required"
                .to_string(),
        ),
        "snapshot_write" | "checkpoint_write" | "progress_artifact_write" => Some(
            "write artifacts less frequently or move report output to faster local storage"
                .to_string(),
        ),
        _ => None,
    }
}

fn rule_group_recommendation(group: &str) -> String {
    format!(
        "review `{}` findings and timing; tune rule config or temporarily disable this group during exploratory performance runs",
        group
    )
}

fn build_audit_performance(
    elapsed_us: u64,
    mut phases: Vec<PhaseTiming>,
    rule_groups: Vec<RuleTiming>,
) -> AuditPerformance {
    let wall_clock_us = elapsed_us;
    let cumulative_tracked_us: u64 = phases
        .iter()
        .filter(|phase| phase.basis == "cumulative")
        .map(|phase| phase.elapsed_us)
        .sum();
    let cumulative_basis_us = cumulative_tracked_us.max(1);
    let wall_basis_us = wall_clock_us.max(1);
    for phase in &mut phases {
        phase.wall_share_basis_points =
            uncapped_share_basis_points(phase.elapsed_us, wall_basis_us);
        phase.cumulative_share_basis_points =
            capped_share_basis_points(phase.elapsed_us, cumulative_basis_us);
    }
    let mut bottlenecks = Vec::new();
    for phase in phases.iter().take(5) {
        let cumulative_share = capped_share_basis_points(phase.elapsed_us, cumulative_basis_us);
        bottlenecks.push(PerformanceBottleneck {
            kind: "phase".to_string(),
            name: phase.name.clone(),
            elapsed_us: phase.elapsed_us,
            share_basis_points: cumulative_share,
            wall_share_basis_points: uncapped_share_basis_points(phase.elapsed_us, wall_basis_us),
            cumulative_share_basis_points: cumulative_share,
            findings: None,
            recommendation: phase_recommendation(&phase.name),
        });
    }
    let total_rule_group_us: u64 = rule_groups.iter().map(|timing| timing.elapsed_us).sum();
    let rule_group_basis_us = total_rule_group_us.max(1);
    for timing in rule_groups.iter().take(5) {
        let cumulative_share = capped_share_basis_points(timing.elapsed_us, cumulative_basis_us);
        bottlenecks.push(PerformanceBottleneck {
            kind: "rule_group".to_string(),
            name: timing.group.clone(),
            elapsed_us: timing.elapsed_us,
            share_basis_points: capped_share_basis_points(timing.elapsed_us, rule_group_basis_us),
            wall_share_basis_points: uncapped_share_basis_points(timing.elapsed_us, wall_basis_us),
            cumulative_share_basis_points: cumulative_share,
            findings: Some(timing.findings),
            recommendation: Some(rule_group_recommendation(&timing.group)),
        });
    }
    bottlenecks.sort_by(|left, right| {
        right
            .elapsed_us
            .cmp(&left.elapsed_us)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.name.cmp(&right.name))
    });
    bottlenecks.truncate(8);

    let mut observations = Vec::new();
    if let Some(top) = bottlenecks.first()
        && top.share_basis_points >= 2_500
    {
        observations.push(format!(
            "{} `{}` accounts for {} of its cost basis",
            top.kind,
            top.name,
            format_share(top.share_basis_points)
        ));
    }
    if let Some(fetch) = phases.iter().find(|phase| phase.name == "fetch")
        && capped_share_basis_points(fetch.elapsed_us, cumulative_basis_us) >= 5_000
    {
        observations.push(
            "fetch dominates cumulative crawler cost; throughput is likely network-bound"
                .to_string(),
        );
    }
    if let Some(rule_eval) = phases.iter().find(|phase| phase.name == "rule_evaluation")
        && uncapped_share_basis_points(rule_eval.elapsed_us, wall_basis_us) >= 2_000
    {
        observations.push(
            "rule evaluation is a material runtime cost; inspect slowest rule groups".to_string(),
        );
    }
    if phases
        .iter()
        .any(|phase| phase.wall_share_basis_points > 10_000 && phase.basis == "cumulative")
    {
        observations.push(
            "one or more cumulative phases exceed wall-clock time; use cumulative shares for cost attribution and wall shares for latency impact"
                .to_string(),
        );
    }

    AuditPerformance {
        elapsed_us,
        wall_clock_us,
        cumulative_tracked_us,
        phases,
        rule_groups,
        bottlenecks,
        observations,
        budget: None,
    }
}

fn budget_violation(
    metric: &str,
    actual: u64,
    budget: u64,
    unit: &str,
) -> PerformanceBudgetViolation {
    PerformanceBudgetViolation {
        metric: metric.to_string(),
        actual,
        budget,
        unit: unit.to_string(),
        message: format!("{metric} actual {actual}{unit} exceeds budget {budget}{unit}"),
    }
}

fn phase_ms(performance: &AuditPerformance, name: &str) -> Option<u64> {
    performance
        .phases
        .iter()
        .find(|phase| phase.name == name)
        .map(|phase| phase.elapsed_us / 1_000)
}

fn phase_p95_ms(performance: &AuditPerformance, name: &str) -> Option<u64> {
    performance
        .phases
        .iter()
        .find(|phase| phase.name == name)
        .and_then(|phase| (phase.p95_us > 0).then_some(phase.p95_us / 1_000))
}

pub fn evaluate_performance_budget(
    artifact: &AuditArtifact,
    budget: PerformanceBudget,
    budget_path: Option<String>,
) -> PerformanceBudgetReport {
    let mut violations = Vec::new();
    let mut warnings = Vec::new();
    let performance = artifact.performance.as_ref();

    if let Some(limit) = budget.max_elapsed_ms {
        let actual = artifact
            .crawl
            .as_ref()
            .map(|crawl| crawl.elapsed_ms)
            .or_else(|| performance.map(|item| item.wall_clock_us / 1_000))
            .unwrap_or_default();
        if actual > limit {
            violations.push(budget_violation("elapsed", actual, limit, "ms"));
        }
    }
    if let Some(limit) = budget.max_fetch_average_ms {
        let actual = artifact
            .crawl
            .as_ref()
            .map(|crawl| crawl.average_fetch_ms)
            .unwrap_or_default();
        if actual > limit {
            violations.push(budget_violation("fetch_average", actual, limit, "ms"));
        }
    }
    if let Some(limit) = budget.max_fetch_p95_ms {
        match performance.and_then(|item| phase_p95_ms(item, "fetch")) {
            Some(actual) if actual > limit => {
                violations.push(budget_violation("fetch_p95", actual, limit, "ms"));
            }
            Some(_) => {}
            None => warnings.push("fetch p95 budget could not be evaluated".to_string()),
        }
    }
    if let Some(limit) = budget.max_rule_evaluation_ms {
        match performance.and_then(|item| phase_ms(item, "rule_evaluation")) {
            Some(actual) if actual > limit => {
                violations.push(budget_violation("rule_evaluation", actual, limit, "ms"));
            }
            Some(_) => {}
            None => warnings.push("rule evaluation budget could not be evaluated".to_string()),
        }
    }
    if let Some(limit) = budget.max_final_audit_ms {
        match performance.and_then(|item| phase_ms(item, "final_audit")) {
            Some(actual) if actual > limit => {
                violations.push(budget_violation("final_audit", actual, limit, "ms"));
            }
            Some(_) => {}
            None => warnings.push("final audit budget could not be evaluated".to_string()),
        }
    }
    if let Some(limit) = budget.max_total_findings {
        let actual = artifact.summary.total as u64;
        if actual > limit as u64 {
            violations.push(budget_violation(
                "total_findings",
                actual,
                limit as u64,
                " findings",
            ));
        }
    }
    if let Some(limit) = budget.max_errors {
        let actual = artifact.summary.errors as u64;
        if actual > limit as u64 {
            violations.push(budget_violation("errors", actual, limit as u64, " errors"));
        }
    }

    PerformanceBudgetReport {
        passed: violations.is_empty(),
        budget_path,
        budget,
        violations,
        warnings,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PerformanceMetricDirection {
    LowerIsBetter,
    HigherIsBetter,
}

impl PerformanceMetricDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::LowerIsBetter => "lower_is_better",
            Self::HigherIsBetter => "higher_is_better",
        }
    }
}

fn relative_delta_basis_points(baseline: u64, current: u64) -> Option<i64> {
    if baseline == 0 {
        return None;
    }
    let delta = current as i128 - baseline as i128;
    Some(((delta * 10_000) / baseline as i128) as i64)
}

fn significant_delta(
    absolute_delta: u64,
    relative_delta_basis_points: Option<i64>,
    relative_threshold_basis_points: u32,
    absolute_threshold: u64,
) -> bool {
    if absolute_delta <= absolute_threshold {
        return false;
    }
    relative_delta_basis_points
        .map(|basis_points| basis_points.unsigned_abs() >= relative_threshold_basis_points as u64)
        .unwrap_or(true)
}

struct PerformanceMetricInput<'a> {
    metric: &'a str,
    label: &'a str,
    unit: &'a str,
    direction: PerformanceMetricDirection,
    baseline: Option<u64>,
    current: Option<u64>,
}

fn build_performance_metric_delta(
    input: PerformanceMetricInput<'_>,
    thresholds: &PerformanceDiffThresholds,
) -> PerformanceMetricDelta {
    let mut delta = None;
    let mut relative_delta = None;
    let mut regressed = false;
    let mut improved = false;

    if let (Some(baseline), Some(current)) = (input.baseline, input.current) {
        let raw_delta = current as i64 - baseline as i64;
        delta = Some(raw_delta);
        relative_delta = relative_delta_basis_points(baseline, current);
        let absolute_delta = raw_delta.unsigned_abs();
        let absolute_threshold = if input.unit == "ms" {
            thresholds.absolute_threshold
        } else {
            0
        };
        let is_significant = significant_delta(
            absolute_delta,
            relative_delta,
            thresholds.relative_threshold_basis_points,
            absolute_threshold,
        );
        match input.direction {
            PerformanceMetricDirection::LowerIsBetter => {
                regressed = raw_delta > 0 && is_significant;
                improved = raw_delta < 0 && is_significant;
            }
            PerformanceMetricDirection::HigherIsBetter => {
                regressed = raw_delta < 0 && is_significant;
                improved = raw_delta > 0 && is_significant;
            }
        }
    }

    let status = if input.baseline.is_none() || input.current.is_none() {
        "missing"
    } else if regressed {
        "regressed"
    } else if improved {
        "improved"
    } else {
        "unchanged"
    };

    PerformanceMetricDelta {
        metric: input.metric.to_string(),
        label: input.label.to_string(),
        unit: input.unit.to_string(),
        direction: input.direction.as_str().to_string(),
        baseline: input.baseline,
        current: input.current,
        delta,
        relative_delta_basis_points: relative_delta,
        status: status.to_string(),
        regressed,
        improved,
    }
}

fn add_performance_metric(
    metrics: &mut Vec<PerformanceMetricDelta>,
    thresholds: &PerformanceDiffThresholds,
    input: PerformanceMetricInput<'_>,
) {
    metrics.push(build_performance_metric_delta(input, thresholds));
}

fn crawl_value(artifact: &AuditArtifact, extract: impl FnOnce(&CrawlStats) -> u64) -> Option<u64> {
    artifact.crawl.as_ref().map(extract)
}

fn performance_value(
    artifact: &AuditArtifact,
    extract: impl FnOnce(&AuditPerformance) -> u64,
) -> Option<u64> {
    artifact.performance.as_ref().map(extract)
}

fn performance_wall_clock_ms(artifact: &AuditArtifact) -> Option<u64> {
    performance_value(artifact, |performance| {
        if performance.wall_clock_us > 0 {
            performance.wall_clock_us / 1_000
        } else {
            performance.elapsed_us / 1_000
        }
    })
}

fn performance_cumulative_tracked_ms(artifact: &AuditArtifact) -> Option<u64> {
    performance_value(artifact, |performance| {
        if performance.cumulative_tracked_us > 0 {
            return performance.cumulative_tracked_us / 1_000;
        }
        performance
            .phases
            .iter()
            .filter(|phase| phase.basis == "cumulative" || phase.basis.is_empty())
            .map(|phase| phase.elapsed_us)
            .sum::<u64>()
            / 1_000
    })
}

fn phase_value(
    artifact: &AuditArtifact,
    phase_name: &str,
    extract: impl FnOnce(&PhaseTiming) -> Option<u64>,
) -> Option<u64> {
    artifact
        .performance
        .as_ref()
        .and_then(|performance| {
            performance
                .phases
                .iter()
                .find(|phase| phase.name == phase_name)
        })
        .and_then(extract)
}

fn rule_group_value(
    artifact: &AuditArtifact,
    group_name: &str,
    extract: impl FnOnce(&RuleTiming) -> u64,
) -> Option<u64> {
    artifact
        .performance
        .as_ref()
        .and_then(|performance| {
            performance
                .rule_groups
                .iter()
                .find(|group| group.group == group_name)
        })
        .map(extract)
}

fn collect_phase_names(baseline: &AuditArtifact, current: &AuditArtifact) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for artifact in [baseline, current] {
        if let Some(performance) = artifact.performance.as_ref() {
            for phase in &performance.phases {
                names.insert(phase.name.clone());
            }
        }
    }
    names
}

fn collect_rule_group_names(baseline: &AuditArtifact, current: &AuditArtifact) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for artifact in [baseline, current] {
        if let Some(performance) = artifact.performance.as_ref() {
            for group in &performance.rule_groups {
                names.insert(group.group.clone());
            }
        }
    }
    names
}

fn performance_diff_warnings(baseline: &AuditArtifact, current: &AuditArtifact) -> Vec<String> {
    let mut warnings = Vec::new();
    if baseline.crawl.is_none() || current.crawl.is_none() {
        warnings.push(
            "one or both artifacts do not include crawl stats; crawl-level metrics may be missing"
                .to_string(),
        );
    }
    if baseline.performance.is_none() || current.performance.is_none() {
        warnings.push(
            "one or both artifacts do not include performance metadata; phase-level metrics may be missing"
                .to_string(),
        );
    }
    if baseline.status != current.status {
        warnings.push(format!(
            "audit statuses differ: baseline={:?} current={:?}",
            baseline.status, current.status
        ));
    }
    if let (Some(baseline_crawl), Some(current_crawl)) = (&baseline.crawl, &current.crawl) {
        if baseline_crawl.engine != current_crawl.engine {
            warnings.push(format!(
                "runtime engines differ: baseline={} current={}",
                baseline_crawl.engine, current_crawl.engine
            ));
        }
        if baseline_crawl.visited_pages != current_crawl.visited_pages {
            warnings.push(format!(
                "visited page counts differ: baseline={} current={}; normalize seeds and max-pages before interpreting absolute timing deltas",
                baseline_crawl.visited_pages, current_crawl.visited_pages
            ));
        }
        if baseline_crawl.max_pages != current_crawl.max_pages {
            warnings.push(format!(
                "max-pages differ: baseline={} current={}",
                baseline_crawl.max_pages, current_crawl.max_pages
            ));
        }
        if baseline_crawl.truncated || current_crawl.truncated {
            warnings.push(
                "one or both crawls were truncated; compare a complete run before setting hard budgets"
                    .to_string(),
            );
        }
    }
    warnings
}

fn summarize_performance_metrics(metrics: &[PerformanceMetricDelta]) -> PerformanceDiffSummary {
    let mut summary = PerformanceDiffSummary::default();
    for metric in metrics {
        match metric.status.as_str() {
            "regressed" => {
                summary.metrics_compared += 1;
                summary.regressions += 1;
            }
            "improved" => {
                summary.metrics_compared += 1;
                summary.improvements += 1;
            }
            "unchanged" => {
                summary.metrics_compared += 1;
                summary.unchanged += 1;
            }
            _ => summary.missing += 1,
        }
    }
    summary
}

pub fn diff_performance_artifacts(
    baseline: &AuditArtifact,
    current: &AuditArtifact,
    baseline_path: Option<String>,
    current_path: Option<String>,
    thresholds: PerformanceDiffThresholds,
) -> PerformanceDiffReport {
    let mut metrics = Vec::new();
    add_performance_metric(
        &mut metrics,
        &thresholds,
        PerformanceMetricInput {
            metric: "crawl.elapsed_ms",
            label: "Crawl elapsed",
            unit: "ms",
            direction: PerformanceMetricDirection::LowerIsBetter,
            baseline: crawl_value(baseline, |crawl| crawl.elapsed_ms),
            current: crawl_value(current, |crawl| crawl.elapsed_ms),
        },
    );
    add_performance_metric(
        &mut metrics,
        &thresholds,
        PerformanceMetricInput {
            metric: "crawl.pages_per_minute",
            label: "Crawl throughput",
            unit: "pages/min",
            direction: PerformanceMetricDirection::HigherIsBetter,
            baseline: crawl_value(baseline, |crawl| crawl.pages_per_minute as u64),
            current: crawl_value(current, |crawl| crawl.pages_per_minute as u64),
        },
    );
    add_performance_metric(
        &mut metrics,
        &thresholds,
        PerformanceMetricInput {
            metric: "crawl.average_fetch_ms",
            label: "Average fetch",
            unit: "ms",
            direction: PerformanceMetricDirection::LowerIsBetter,
            baseline: crawl_value(baseline, |crawl| crawl.average_fetch_ms),
            current: crawl_value(current, |crawl| crawl.average_fetch_ms),
        },
    );
    add_performance_metric(
        &mut metrics,
        &thresholds,
        PerformanceMetricInput {
            metric: "crawl.average_page_process_ms",
            label: "Average page processing",
            unit: "ms",
            direction: PerformanceMetricDirection::LowerIsBetter,
            baseline: crawl_value(baseline, |crawl| crawl.average_page_process_ms),
            current: crawl_value(current, |crawl| crawl.average_page_process_ms),
        },
    );
    add_performance_metric(
        &mut metrics,
        &thresholds,
        PerformanceMetricInput {
            metric: "crawl.average_partial_audit_ms",
            label: "Average partial audit",
            unit: "ms",
            direction: PerformanceMetricDirection::LowerIsBetter,
            baseline: crawl_value(baseline, |crawl| crawl.average_partial_audit_ms),
            current: crawl_value(current, |crawl| crawl.average_partial_audit_ms),
        },
    );
    add_performance_metric(
        &mut metrics,
        &thresholds,
        PerformanceMetricInput {
            metric: "crawl.fetch_failures",
            label: "Fetch failures",
            unit: "count",
            direction: PerformanceMetricDirection::LowerIsBetter,
            baseline: crawl_value(baseline, |crawl| crawl.fetch_failures as u64),
            current: crawl_value(current, |crawl| crawl.fetch_failures as u64),
        },
    );
    add_performance_metric(
        &mut metrics,
        &thresholds,
        PerformanceMetricInput {
            metric: "crawl.fetch_retries",
            label: "Fetch retries",
            unit: "count",
            direction: PerformanceMetricDirection::LowerIsBetter,
            baseline: crawl_value(baseline, |crawl| crawl.fetch_retries as u64),
            current: crawl_value(current, |crawl| crawl.fetch_retries as u64),
        },
    );
    add_performance_metric(
        &mut metrics,
        &thresholds,
        PerformanceMetricInput {
            metric: "performance.wall_clock_ms",
            label: "Performance wall clock",
            unit: "ms",
            direction: PerformanceMetricDirection::LowerIsBetter,
            baseline: performance_wall_clock_ms(baseline),
            current: performance_wall_clock_ms(current),
        },
    );
    add_performance_metric(
        &mut metrics,
        &thresholds,
        PerformanceMetricInput {
            metric: "performance.cumulative_tracked_ms",
            label: "Cumulative tracked time",
            unit: "ms",
            direction: PerformanceMetricDirection::LowerIsBetter,
            baseline: performance_cumulative_tracked_ms(baseline),
            current: performance_cumulative_tracked_ms(current),
        },
    );
    add_performance_metric(
        &mut metrics,
        &thresholds,
        PerformanceMetricInput {
            metric: "summary.total_findings",
            label: "Total findings",
            unit: "count",
            direction: PerformanceMetricDirection::LowerIsBetter,
            baseline: Some(baseline.summary.total as u64),
            current: Some(current.summary.total as u64),
        },
    );
    add_performance_metric(
        &mut metrics,
        &thresholds,
        PerformanceMetricInput {
            metric: "summary.errors",
            label: "Error findings",
            unit: "count",
            direction: PerformanceMetricDirection::LowerIsBetter,
            baseline: Some(baseline.summary.errors as u64),
            current: Some(current.summary.errors as u64),
        },
    );

    for phase_name in collect_phase_names(baseline, current) {
        let metric_name = format!("phase.{}.elapsed_ms", phase_name);
        let label = format!("Phase `{}` elapsed", phase_name);
        add_performance_metric(
            &mut metrics,
            &thresholds,
            PerformanceMetricInput {
                metric: &metric_name,
                label: &label,
                unit: "ms",
                direction: PerformanceMetricDirection::LowerIsBetter,
                baseline: phase_value(baseline, &phase_name, |phase| {
                    Some(phase.elapsed_us / 1_000)
                }),
                current: phase_value(current, &phase_name, |phase| Some(phase.elapsed_us / 1_000)),
            },
        );
        let baseline_p95 = phase_value(baseline, &phase_name, |phase| {
            (phase.p95_us > 0).then_some(phase.p95_us / 1_000)
        });
        let current_p95 = phase_value(current, &phase_name, |phase| {
            (phase.p95_us > 0).then_some(phase.p95_us / 1_000)
        });
        if baseline_p95.is_some() || current_p95.is_some() {
            let p95_metric_name = format!("phase.{}.p95_ms", phase_name);
            let p95_label = format!("Phase `{}` p95", phase_name);
            add_performance_metric(
                &mut metrics,
                &thresholds,
                PerformanceMetricInput {
                    metric: &p95_metric_name,
                    label: &p95_label,
                    unit: "ms",
                    direction: PerformanceMetricDirection::LowerIsBetter,
                    baseline: baseline_p95,
                    current: current_p95,
                },
            );
        }
    }

    for group_name in collect_rule_group_names(baseline, current) {
        let metric_name = format!("rule_group.{}.elapsed_ms", group_name);
        let label = format!("Rule group `{}` elapsed", group_name);
        add_performance_metric(
            &mut metrics,
            &thresholds,
            PerformanceMetricInput {
                metric: &metric_name,
                label: &label,
                unit: "ms",
                direction: PerformanceMetricDirection::LowerIsBetter,
                baseline: rule_group_value(baseline, &group_name, |group| group.elapsed_us / 1_000),
                current: rule_group_value(current, &group_name, |group| group.elapsed_us / 1_000),
            },
        );
    }

    let summary = summarize_performance_metrics(&metrics);
    let warnings = performance_diff_warnings(baseline, current);
    PerformanceDiffReport {
        baseline_path,
        current_path,
        thresholds,
        summary,
        metrics,
        warnings,
    }
}

fn format_performance_metric_value(value: Option<u64>, unit: &str) -> String {
    match value {
        Some(value) if unit == "ms" => format!("{value}ms"),
        Some(value) if unit == "pages/min" => format!("{value} pages/min"),
        Some(value) => format!("{value} {unit}"),
        None => "n/a".to_string(),
    }
}

fn format_signed_delta(value: Option<i64>, unit: &str) -> String {
    match value {
        Some(value) if unit == "ms" => format!("{value:+}ms"),
        Some(value) if unit == "pages/min" => format!("{value:+} pages/min"),
        Some(value) => format!("{value:+} {unit}"),
        None => "n/a".to_string(),
    }
}

fn format_signed_basis_points(value: Option<i64>) -> String {
    match value {
        Some(value) => {
            let sign = if value >= 0 { "+" } else { "-" };
            let abs = value.unsigned_abs();
            format!("{sign}{}.{:02}%", abs / 100, abs % 100)
        }
        None => "n/a".to_string(),
    }
}

fn format_unsigned_basis_points(value: u32) -> String {
    format!("{}.{:02}%", value / 100, value % 100)
}

pub fn render_performance_diff_text(report: &PerformanceDiffReport) -> String {
    let mut lines = vec!["Performance Diff".to_string(), String::new()];
    if let Some(path) = report.baseline_path.as_deref() {
        lines.push(format!("Baseline: {path}"));
    }
    if let Some(path) = report.current_path.as_deref() {
        lines.push(format!("Current: {path}"));
    }
    lines.push(format!(
        "Thresholds: relative={} absolute={}ms",
        format_unsigned_basis_points(report.thresholds.relative_threshold_basis_points),
        report.thresholds.absolute_threshold
    ));
    lines.push(format!(
        "Summary: regressions={} improvements={} unchanged={} missing={}",
        report.summary.regressions,
        report.summary.improvements,
        report.summary.unchanged,
        report.summary.missing
    ));
    lines.push(String::new());
    lines.push("Metrics".to_string());
    for metric in &report.metrics {
        lines.push(format!(
            "- {}: {} -> {} ({}, {}) {}",
            metric.metric,
            format_performance_metric_value(metric.baseline, &metric.unit),
            format_performance_metric_value(metric.current, &metric.unit),
            format_signed_delta(metric.delta, &metric.unit),
            format_signed_basis_points(metric.relative_delta_basis_points),
            metric.status
        ));
    }
    if !report.warnings.is_empty() {
        lines.push(String::new());
        lines.push("Warnings".to_string());
        for warning in &report.warnings {
            lines.push(format!("- {warning}"));
        }
    }
    lines.join("\n")
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
    artifact.performance = Some(build_audit_performance(
        crawl_stats.elapsed_us,
        performance.phase_timings(crawl_stats),
        Vec::new(),
    ));
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
    let mut phases = performance.phase_timings(crawl_stats);
    phases.push(PhaseTiming {
        name: "partial_snapshot_build".to_string(),
        elapsed_us: snapshot_build_us,
        basis: "cumulative".to_string(),
        sample_count: 1,
        min_us: snapshot_build_us,
        max_us: snapshot_build_us,
        p50_us: snapshot_build_us,
        p75_us: snapshot_build_us,
        p95_us: snapshot_build_us,
        p99_us: snapshot_build_us,
        ..PhaseTiming::default()
    });
    phases.sort_by(|left, right| right.elapsed_us.cmp(&left.elapsed_us));
    artifact.performance = Some(build_audit_performance(
        crawl_stats.elapsed_us,
        phases,
        profiled.rule_timings,
    ));
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
    let performance_summary = build_audit_performance(
        crawl_stats.elapsed_us,
        performance.phase_timings(&crawl_stats),
        profiled.rule_timings,
    );
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
        RuntimeArtifactMode, RuntimeAuditOptions, RuntimeProgressMode, diff_performance_artifacts,
        evaluate_performance_budget, render_performance_diff_text, run_runtime_audit,
        run_runtime_audit_with_options, verify_runtime_audit,
    };
    use crate::config::{Config, default_rule_switches};
    use aexeo_contracts::{
        AuditArtifact, AuditStatus, CrawlStats, Finding, FindingScope, PerformanceBudget,
        PerformanceDiffThresholds, PhaseTiming, RuleTiming,
    };
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
            "surfaces",
            "content",
            "structure",
        ] {
            config.checks.insert(key.to_string(), false);
        }
        config
    }

    #[test]
    fn performance_summary_identifies_bottlenecks() {
        let performance = super::build_audit_performance(
            10_000,
            vec![
                PhaseTiming {
                    name: "fetch".to_string(),
                    elapsed_us: 7_000,
                    basis: "cumulative".to_string(),
                    ..PhaseTiming::default()
                },
                PhaseTiming {
                    name: "rule_evaluation".to_string(),
                    elapsed_us: 2_000,
                    basis: "nested".to_string(),
                    ..PhaseTiming::default()
                },
            ],
            vec![RuleTiming {
                group: "surfaces".to_string(),
                elapsed_us: 1_500,
                findings: 3,
            }],
        );
        assert_eq!(performance.bottlenecks[0].name, "fetch");
        assert_eq!(performance.bottlenecks[0].share_basis_points, 10_000);
        assert_eq!(performance.bottlenecks[0].wall_share_basis_points, 7_000);
        assert_eq!(performance.phases[0].cumulative_share_basis_points, 10_000);
        assert!(
            performance
                .observations
                .iter()
                .any(|observation| observation.contains("fetch dominates"))
        );
        assert!(
            performance
                .bottlenecks
                .iter()
                .any(|bottleneck| bottleneck.kind == "rule_group" && bottleneck.name == "surfaces")
        );
    }

    #[test]
    fn performance_budget_reports_violations() {
        let mut artifact = AuditArtifact {
            command: "crawl".to_string(),
            crawl: Some(CrawlStats {
                elapsed_ms: 250,
                average_fetch_ms: 80,
                ..CrawlStats::default()
            }),
            ..AuditArtifact::default()
        };
        artifact.summary.total = 3;
        artifact.performance = Some(super::build_audit_performance(
            250_000,
            vec![PhaseTiming {
                name: "fetch".to_string(),
                elapsed_us: 160_000,
                basis: "cumulative".to_string(),
                sample_count: 2,
                p95_us: 120_000,
                ..PhaseTiming::default()
            }],
            Vec::new(),
        ));
        let report = evaluate_performance_budget(
            &artifact,
            PerformanceBudget {
                max_elapsed_ms: Some(200),
                max_fetch_average_ms: Some(70),
                max_fetch_p95_ms: Some(100),
                max_total_findings: Some(2),
                ..PerformanceBudget::default()
            },
            Some("budget.json".to_string()),
        );
        assert!(!report.passed);
        assert_eq!(report.violations.len(), 4);
        assert!(
            report
                .violations
                .iter()
                .any(|item| item.metric == "fetch_p95")
        );
    }

    #[test]
    fn performance_diff_marks_significant_regressions() {
        let baseline = AuditArtifact {
            command: "crawl".to_string(),
            crawl: Some(CrawlStats {
                engine: "http".to_string(),
                visited_pages: 10,
                max_pages: 10,
                elapsed_ms: 100,
                pages_per_minute: 600,
                average_fetch_ms: 20,
                average_page_process_ms: 4,
                ..CrawlStats::default()
            }),
            performance: Some(super::build_audit_performance(
                100_000,
                vec![PhaseTiming {
                    name: "fetch".to_string(),
                    elapsed_us: 50_000,
                    basis: "cumulative".to_string(),
                    sample_count: 2,
                    p95_us: 30_000,
                    ..PhaseTiming::default()
                }],
                vec![RuleTiming {
                    group: "schema".to_string(),
                    elapsed_us: 10_000,
                    findings: 1,
                }],
            )),
            ..AuditArtifact::default()
        };
        let current = AuditArtifact {
            command: "crawl".to_string(),
            crawl: Some(CrawlStats {
                engine: "http".to_string(),
                visited_pages: 11,
                max_pages: 10,
                elapsed_ms: 140,
                pages_per_minute: 420,
                average_fetch_ms: 40,
                average_page_process_ms: 4,
                ..CrawlStats::default()
            }),
            performance: Some(super::build_audit_performance(
                140_000,
                vec![PhaseTiming {
                    name: "fetch".to_string(),
                    elapsed_us: 80_000,
                    basis: "cumulative".to_string(),
                    sample_count: 2,
                    p95_us: 60_000,
                    ..PhaseTiming::default()
                }],
                vec![RuleTiming {
                    group: "schema".to_string(),
                    elapsed_us: 20_000,
                    findings: 1,
                }],
            )),
            ..AuditArtifact::default()
        };

        let report = diff_performance_artifacts(
            &baseline,
            &current,
            Some("baseline.json".to_string()),
            Some("current.json".to_string()),
            PerformanceDiffThresholds {
                relative_threshold_basis_points: 1_000,
                absolute_threshold: 0,
            },
        );

        assert!(report.summary.regressions >= 4);
        assert!(
            report
                .metrics
                .iter()
                .any(|metric| metric.metric == "phase.fetch.p95_ms" && metric.regressed)
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("visited page counts differ"))
        );
        let text = render_performance_diff_text(&report);
        assert!(text.contains("Performance Diff"));
        assert!(text.contains("phase.fetch.p95_ms"));
    }

    #[test]
    fn performance_diff_falls_back_for_legacy_timing_fields() {
        let artifact = AuditArtifact {
            command: "crawl".to_string(),
            performance: Some(aexeo_contracts::AuditPerformance {
                elapsed_us: 123_000,
                phases: vec![PhaseTiming {
                    name: "fetch".to_string(),
                    elapsed_us: 50_000,
                    ..PhaseTiming::default()
                }],
                ..aexeo_contracts::AuditPerformance::default()
            }),
            ..AuditArtifact::default()
        };

        let report = diff_performance_artifacts(
            &artifact,
            &artifact,
            None,
            None,
            PerformanceDiffThresholds::default(),
        );

        let wall_clock = report
            .metrics
            .iter()
            .find(|metric| metric.metric == "performance.wall_clock_ms")
            .unwrap();
        assert_eq!(wall_clock.baseline, Some(123));
        let cumulative = report
            .metrics
            .iter()
            .find(|metric| metric.metric == "performance.cumulative_tracked_ms")
            .unwrap();
        assert_eq!(cumulative.baseline, Some(50));
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
                sitemap_text: None,
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
            |artifact: &aexeo_contracts::AuditArtifact| -> anyhow::Result<()> {
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
