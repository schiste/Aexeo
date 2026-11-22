//! Manual end-to-end benchmark for inventory-alias reconciliation.
//! Usage: `cargo run --release -p aexeo-core --example link_alias_bench -- [aliases] [iterations]`.
//! Fixture creation is excluded; the entire link report is measured.
use aexeo_core::link_inventory::{InventoryDisposition, InventoryPage, InventorySnapshot};
use aexeo_core::link_report::{LinkReportInput, RenderedPage, build_link_report};
use std::{hint::black_box, time::Instant};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let count: usize = args.next().as_deref().unwrap_or("2000").parse()?;
    let iterations: usize = args.next().as_deref().unwrap_or("5").parse()?;
    anyhow::ensure!(count > 0 && iterations > 0, "counts must be positive");
    let mut pages = Vec::with_capacity(count + 1);
    let mut inventory = Vec::with_capacity(count);
    for i in 0..count {
        let url = format!("https://example.org/alias-{i}");
        pages.push(RenderedPage {
            url: url.clone(),
            html: "<html lang='en'><head><link rel='canonical' href='/target'></head><body></body></html>".into(),
            status_code: Some(200),
            response_headers: Some(Default::default()),
            requested_url: None,
            redirect_chain: Vec::new(),
            redirected_arrivals: Vec::new(),
        });
        inventory.push(InventoryPage {
            url,
            disposition: InventoryDisposition::CanonicalAlias,
            target: Some("/target".into()),
            locale: Some("en".into()),
        });
    }
    pages.push(RenderedPage {
        url: "https://example.org/target".into(),
        html: "<html lang='en'><body></body></html>".into(),
        status_code: Some(200),
        response_headers: Some(Default::default()),
        requested_url: None,
        redirect_chain: Vec::new(),
        redirected_arrivals: Vec::new(),
    });
    let input = LinkReportInput {
        version: 1,
        identity: None,
        snapshot_date: None,
        crawl_complete: false,
        pages,
        inventory: Some(InventorySnapshot {
            identity: None,
            complete: false,
            pages: inventory,
        }),
    };
    let warmup = build_link_report(&input)?;
    assert_eq!(
        warmup
            .pages
            .iter()
            .map(|p| p.inventory_aliases.len())
            .sum::<usize>(),
        count
    );
    let started = Instant::now();
    for _ in 0..iterations {
        black_box(build_link_report(black_box(&input))?);
    }
    println!(
        "aliases={count} iterations={iterations} avg_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0 / iterations as f64
    );
    Ok(())
}
