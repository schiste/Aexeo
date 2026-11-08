//! In-process rendered-page graph benchmark. Fixture creation is excluded.
//!
//! Usage: `link_graph_bench [--json] [ITERATIONS]`. Each tier is warmed up,
//! then timed for ITERATIONS runs (default 30). With `--json` it prints one
//! JSON object per tier, which `scripts/check-performance.sh` compares
//! against `performance-budget.json`.
use aexeo_core::link_inventory::SurfaceIdentity;
use aexeo_core::link_report::{LinkReportInput, RenderedPage, build_link_report};
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
    for tier in &TIERS {
        let result = run_tier(tier, iterations)?;
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
