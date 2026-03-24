from __future__ import annotations

"""Runtime crawl and verification orchestration."""

from dataclasses import dataclass
from pathlib import Path

from seogeo.config import Config
from seogeo.crawl import crawl_site
from seogeo.engine import run_checks_for_site
from seogeo.models import Finding, Site
from seogeo.verification import diff_finding_sets, load_findings_from_audit


@dataclass(slots=True, frozen=True)
class CrawlRequest:
    """Normalized runtime crawl inputs derived from CLI flags and config."""

    url: str
    max_pages: int
    engine: str
    wait_until: str
    artifact_dir: Path


@dataclass(slots=True)
class RuntimeAudit:
    """Structured output of one runtime crawl plus rule execution."""

    site: Site
    crawl_findings: list[Finding]
    findings: list[Finding]


def build_crawl_request(url: str, max_pages: int, engine: str, config: Config) -> CrawlRequest:
    """Build a runtime crawl request using config-backed defaults."""
    return CrawlRequest(
        url=url,
        max_pages=max_pages,
        engine=engine or config.browser_engine,
        wait_until=config.browser_wait_until,
        artifact_dir=Path(config.crawl_artifact_dir),
    )


def run_runtime_audit(request: CrawlRequest, config: Config) -> RuntimeAudit:
    """Run a runtime crawl and then apply the enabled rule groups to the crawled site."""
    site, crawl_findings = crawl_site(
        request.url,
        max_pages=request.max_pages,
        engine=request.engine,
        wait_until=request.wait_until,
        request_headers=config.crawl_headers,
        cookies=config.crawl_cookies,
        basic_auth=config.crawl_basic_auth,
        artifact_dir=request.artifact_dir,
        capture_trace=config.crawl_capture_trace,
        capture_screenshot=config.crawl_capture_screenshot,
        capture_console=config.crawl_capture_console,
        capture_network=config.crawl_capture_network,
        setup_plugins=config.plugins,
        plugin_settings=config.plugin_settings,
    )
    findings = crawl_findings + run_checks_for_site(site, config)
    return RuntimeAudit(site=site, crawl_findings=crawl_findings, findings=findings)


def verify_runtime_audit(audit: RuntimeAudit, baseline_path: Path) -> tuple[list[Finding], list[Finding], list[Finding]]:
    """Compare one runtime audit against a baseline artifact."""
    baseline_findings = load_findings_from_audit(baseline_path) if baseline_path.exists() else []
    diff = diff_finding_sets(baseline_findings, audit.findings)
    return diff.new_findings, diff.resolved_findings, diff.unchanged_findings
