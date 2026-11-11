//! Aggregate source-mapping timings for a long multilingual CMS span.
use aexeo_core::content_gate::ContentPreset;
use aexeo_emdash_bridge::{
    EmdashDocument,
    content::{evaluate_content, source_locations},
};
use std::{hint::black_box, time::Instant};

fn main() -> anyhow::Result<()> {
    let text = format!("{} target ", "é😀".repeat(1024)).repeat(250);
    let document: EmdashDocument = serde_json::from_value(serde_json::json!({
        "route": "/benchmark", "title": "Benchmark", "body": [
            {"children": [{"_type": "span", "text": text}]}
        ]
    }))?;
    let preset = ContentPreset::from_toml(
        "version=1\nname='bench'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['target']\n",
    )?;
    let report = evaluate_content(&document, &preset)?;
    let mut timings = Vec::with_capacity(35);
    for _ in 0..35 {
        let start = Instant::now();
        let locations =
            source_locations(black_box(&document), black_box(&preset), black_box(&report));
        assert_eq!(locations.len(), 250);
        black_box(locations);
        timings.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    timings.drain(..5);
    timings.sort_unstable_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({
            "name": "content/emdash_unicode_source_mapping",
            "avg_ms": timings.iter().sum::<f64>() / timings.len() as f64,
            "p50_ms": timings[14], "p95_ms": timings[28],
            "iterations": timings.len(), "findings": 250,
            "content_bytes": document.body[0].children.iter().map(|child| match child {
                aexeo_emdash_bridge::PortableTextChild::Span(span) => span.text.len(),
            }).sum::<usize>()
        })
    );
    Ok(())
}
