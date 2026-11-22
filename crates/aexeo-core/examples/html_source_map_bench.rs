//! Synthetic local measurements of HTML mapping, with and without masked citations.
use aexeo_core::html_content_adapter::{html_content, html_source_map};
use std::{hint::black_box, time::Instant};

fn main() -> anyhow::Result<()> {
    for (name, paragraph) in [
        (
            "ordinary",
            "<p>A useful paragraph with <a href='/page'>a link</a> and concrete details.</p>",
        ),
        (
            "citations",
            "<p>A useful paragraph.<a href='#cite_note-source'>[1]</a> More details.</p>",
        ),
    ] {
        let source = format!("<article>{}</article>", paragraph.repeat(500));
        let content = html_content("benchmark", &source).map_err(anyhow::Error::msg)?;
        let projected = content.input.text.as_deref().expect("body observed");
        anyhow::ensure!(
            name != "citations" || !projected.contains("[1]"),
            "citation benchmark was not masked"
        );
        let mut times = Vec::with_capacity(100);
        for _ in 0..100 {
            let start = Instant::now();
            let map = html_source_map(black_box(&source), black_box(projected));
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            anyhow::ensure!(map.is_some(), "benchmark mapping unavailable");
            black_box(map);
            times.push(elapsed);
        }
        times.sort_unstable_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({"schema": "aexeo.html-source-map-benchmark/v1", "case": name, "input_bytes": source.len(), "iterations": times.len(), "avg_ms": times.iter().sum::<f64>() / times.len() as f64, "p50_ms": times[49], "p95_ms": times[94]})
        );
    }
    Ok(())
}
