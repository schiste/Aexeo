//! In-process rendered-page graph benchmark. Fixture creation is excluded.
//!
//! Usage: `link_graph_bench [--json] [ITERATIONS]`. Each tier is warmed up,
//! then timed for ITERATIONS runs (default 30). With `--json` it prints one
//! JSON object per tier, which `scripts/check-performance.sh` compares
//! against `performance-budget.json`.
use aexeo_contracts::{AuditPageSnapshot, Finding};
use aexeo_core::link_evidence::extract_links;
use aexeo_core::link_findings::qualify_legacy_link_findings;
use aexeo_core::link_inventory::SurfaceIdentity;
use aexeo_core::link_report::{LinkReportInput, RenderedPage, build_link_report};
use std::collections::BTreeMap;
use std::{hint::black_box, time::Instant};

const WARMUP_ITERATIONS: usize = 3;

/// A ring of `pages` pages where each links to the next `links_per_page`
/// pages, so every page has exactly `links_per_page` distinct sources.
struct Tier {
    name: &'static str,
    pages: usize,
    links_per_page: usize,
}

const TIERS: [Tier; 2] = [
    Tier {
        name: "link_graph/rendered_50_pages_500_links",
        pages: 50,
        links_per_page: 10,
    },
    Tier {
        name: "link_graph/rendered_1000_pages_20000_links",
        pages: 1000,
        links_per_page: 20,
    },
];

fn fixture(tier: &Tier) -> LinkReportInput {
    let pages = (0..tier.pages)
        .map(|index| {
            let links = (1..=tier.links_per_page)
                .map(|offset| {
                    format!(
                        "<p><a href='/page-{}'>Link</a></p>",
                        (index + offset) % tier.pages
                    )
                })
                .collect::<String>();
            RenderedPage {
                url: format!("https://example.org/page-{index}"),
                html: format!("<html lang='en'><body><main>{links}</main></body></html>"),
                status_code: Some(200),
                response_headers: Some(Default::default()),
                requested_url: None,
                redirect_chain: Vec::new(),
                redirected_arrivals: Vec::new(),
            }
        })
        .collect();
    LinkReportInput {
        version: 1,
        identity: Some(SurfaceIdentity {
            origin: "https://example.org".into(),
            release: "benchmark".into(),
            surface: "benchmark".into(),
            snapshot: "benchmark".into(),
        }),
        snapshot_date: Some("2026-01-01".into()),
        crawl_complete: false,
        pages,
        inventory: None,
    }
}

/// Nearest-rank percentile of an ascending, non-empty sample set.
fn percentile_ms(sorted_us: &[u128], percentile: usize) -> f64 {
    let rank = (sorted_us.len() * percentile).div_ceil(100).max(1);
    sorted_us[rank - 1] as f64 / 1000.0
}

fn run_tier(tier: &Tier, iterations: usize) -> anyhow::Result<serde_json::Value> {
    let input = fixture(tier);
    let warmup = build_link_report(&input)?;
    assert_eq!(warmup.pages.len(), tier.pages);
    assert!(
        warmup
            .pages
            .iter()
            .all(|page| page.distinct_source_pages == tier.links_per_page)
    );
    for _ in 1..WARMUP_ITERATIONS {
        black_box(build_link_report(black_box(&input))?);
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        black_box(build_link_report(black_box(&input))?);
        samples.push(started.elapsed().as_micros());
    }
    samples.sort_unstable();
    let avg_ms = samples.iter().sum::<u128>() as f64 / samples.len() as f64 / 1000.0;
    Ok(serde_json::json!({
        "name": tier.name,
        "warmup_iterations": WARMUP_ITERATIONS,
        "iterations": samples.len(),
        "pages": tier.pages,
        "links": tier.pages * tier.links_per_page,
        "avg_ms": avg_ms,
        "p50_ms": percentile_ms(&samples, 50),
        "p95_ms": percentile_ms(&samples, 95),
    }))
}

/// Exercise a long paragraph with many inline elements and few links. The
/// paragraph context must not be copied for every traversed element.
fn run_dense_paragraph(iterations: usize) -> serde_json::Value {
    let html = format!(
        "<main><p>{}<a href='/target'>target</a></p></main>",
        "<span>word </span>".repeat(4_000)
    );
    let extract = || extract_links(&html, "https://example.org/", "https://example.org", None);
    for _ in 0..WARMUP_ITERATIONS {
        let links = extract();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].paragraph.as_ref().unwrap().len(), 20_006);
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        black_box(extract());
        samples.push(started.elapsed().as_micros());
    }
    samples.sort_unstable();
    serde_json::json!({
        "name": "link_graph/dense_paragraph_4000_spans",
        "warmup_iterations": WARMUP_ITERATIONS,
        "iterations": samples.len(),
        "pages": 1,
        "links": 1,
        "avg_ms": samples.iter().sum::<u128>() as f64 / samples.len() as f64 / 1000.0,
        "p50_ms": percentile_ms(&samples, 50),
        "p95_ms": percentile_ms(&samples, 95),
    })
}

/// Measure qualification itself, not graph construction, with two findings
/// per page. Fixture creation is outside the measured region.
fn run_qualification(iterations: usize) -> anyhow::Result<serde_json::Value> {
    let tier = &TIERS[1];
    let report = build_link_report(&fixture(tier))?;
    let mut urls = BTreeMap::new();
    let mut pages = Vec::new();
    let mut findings = Vec::new();
    for index in 0..tier.pages {
        let path = format!("page-{index}.html");
        urls.insert(path.clone(), format!("https://example.org/page-{index}"));
        pages.push(AuditPageSnapshot {
            path: path.clone(),
            relative_path: path.clone(),
            ..Default::default()
        });
        for rule_id in ["LNK002", "LNK004"] {
            findings.push(Finding {
                rule_id: rule_id.into(),
                message: String::new(),
                path: path.clone(),
                line: 1,
                column: 1,
                severity: "warning".into(),
                suggestion: None,
                scope: Default::default(),
                subject: None,
            });
        }
    }
    for _ in 0..WARMUP_ITERATIONS {
        assert_eq!(
            qualify_legacy_link_findings(&findings, &pages, Some(&urls), &report).len(),
            findings.len()
        );
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        black_box(qualify_legacy_link_findings(
            black_box(&findings),
            black_box(&pages),
            Some(black_box(&urls)),
            black_box(&report),
        ));
        samples.push(started.elapsed().as_micros());
    }
    samples.sort_unstable();
    Ok(serde_json::json!({
        "name": "link_graph/qualify_1000_pages_2000_findings",
        "warmup_iterations": WARMUP_ITERATIONS,
        "iterations": samples.len(),
        "pages": tier.pages,
        "links": tier.pages * tier.links_per_page,
        "findings": findings.len(),
        "avg_ms": samples.iter().sum::<u128>() as f64 / samples.len() as f64 / 1000.0,
        "p50_ms": percentile_ms(&samples, 50),
        "p95_ms": percentile_ms(&samples, 95),
    }))
}

fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let json_mode = args.iter().any(|arg| arg == "--json");
    let iterations = args
        .iter()
        .find(|arg| arg.as_str() != "--json")
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(30)
        .max(1);
    let results = TIERS
        .iter()
        .map(|tier| run_tier(tier, iterations))
        .chain(std::iter::once_with(|| run_qualification(iterations)))
        .chain(std::iter::once_with(|| Ok(run_dense_paragraph(iterations))));
    for result in results {
        let result = result?;
        if json_mode {
            println!("{result}");
        } else {
            println!(
                "{}: pages={} links={} iterations={} avg_ms={:.3} p50_ms={:.3} p95_ms={:.3}",
                result["name"].as_str().unwrap_or_default(),
                result["pages"],
                result["links"],
                result["iterations"],
                result["avg_ms"].as_f64().unwrap_or_default(),
                result["p50_ms"].as_f64().unwrap_or_default(),
                result["p95_ms"].as_f64().unwrap_or_default(),
            );
        }
    }
    Ok(())
}
