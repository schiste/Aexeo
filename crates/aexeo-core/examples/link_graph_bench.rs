//! In-process rendered-page graph benchmark. Fixture creation is excluded.
use aexeo_core::link_inventory::SurfaceIdentity;
use aexeo_core::link_report::{LinkReportInput, RenderedPage, build_link_report};
use std::{hint::black_box, time::Instant};

fn main() -> anyhow::Result<()> {
    let pages = (0..50)
        .map(|index| {
            let links = (1..=10)
                .map(|offset| format!("<p><a href='/page-{}'>Link</a></p>", (index + offset) % 50))
                .collect::<String>();
            RenderedPage {
                url: format!("https://example.org/page-{index}"),
                html: format!("<html lang='en'><body><main>{links}</main></body></html>"),
                status_code: Some(200),
                response_headers: Some(Default::default()),
                requested_url: None,
                redirect_chain: Vec::new(),
            }
        })
        .collect();
    let input = LinkReportInput {
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
    };
    let warmup = build_link_report(&input)?;
    assert_eq!(warmup.pages.len(), 50);
    assert!(
        warmup
            .pages
            .iter()
            .all(|page| page.distinct_source_pages == 10)
    );
    let mut samples = Vec::with_capacity(30);
    for _ in 0..30 {
        let started = Instant::now();
        black_box(build_link_report(black_box(&input))?);
        samples.push(started.elapsed().as_micros());
    }
    samples.sort_unstable();
    let avg_ms = samples.iter().sum::<u128>() as f64 / samples.len() as f64 / 1000.0;
    println!(
        "{}",
        serde_json::json!({
            "name": "link_graph/rendered_50_pages_500_links",
            "iterations": samples.len(),
            "pages": 50,
            "links": 500,
            "avg_ms": avg_ms,
            "p50_ms": samples[14] as f64 / 1000.0,
            "p95_ms": samples[28] as f64 / 1000.0,
        })
    );
    Ok(())
}
