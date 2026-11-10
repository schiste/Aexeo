//! Aggregate EmDash projection and evaluation timings on a synthetic document.
use aexeo_core::content_gate::ContentPreset;
use aexeo_emdash_bridge::{
    EmdashDocument,
    content::{content_input, evaluate_content},
};
use std::{hint::black_box, time::Instant};

fn main() -> anyhow::Result<()> {
    let blocks: Vec<_> = (0..500)
        .map(|index| {
            serde_json::json!({
                "style": if index % 20 == 0 { "h2" } else { "normal" },
                "children": [
                    {"_type":"span", "text":"A short paragraph with "},
                    {"_type":"span", "text":"practical details and examples."}
                ]
            })
        })
        .collect();
    let document: EmdashDocument = serde_json::from_value(serde_json::json!({
        "route": "/benchmark", "title": "Benchmark", "body": blocks
    }))?;
    let preset = ContentPreset::from_toml(
        "version=1\nname='bench'\n[[gates]]\nid='words'\nkind='word_count'\nmin=10\n[[gates]]\nid='phrases'\nkind='forbidden_terms'\nterms=['forbidden','exaggerated']\n",
    )?;
    let mut projection = Vec::with_capacity(105);
    let mut evaluation = Vec::with_capacity(105);
    for _ in 0..105 {
        let start = Instant::now();
        black_box(content_input(black_box(&document)));
        let projection_time = start.elapsed();
        let start = Instant::now();
        black_box(evaluate_content(black_box(&document), black_box(&preset))?);
        let evaluation_time = start.elapsed();
        projection.push(projection_time.as_secs_f64() * 1000.0);
        evaluation.push(evaluation_time.as_secs_f64() * 1000.0);
    }
    for (name, samples) in [
        ("content/emdash_projection", &mut projection),
        ("content/emdash_projection_and_evaluation", &mut evaluation),
    ] {
        samples.drain(..5);
        samples.sort_unstable_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({
                "name": name,
                "avg_ms": samples.iter().sum::<f64>() / samples.len() as f64,
                "p50_ms": samples[49], "p95_ms": samples[94],
                "iterations": samples.len(), "blocks": document.body.len()
            })
        );
    }
    Ok(())
}
