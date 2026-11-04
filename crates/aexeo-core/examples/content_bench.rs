//! Reproducible local benchmark, not a CI pass/fail threshold.
use aexeo_core::{content_gate::ContentPreset, markdown_adapter::markdown_input};
use std::{hint::black_box, time::Instant};

fn main() -> anyhow::Result<()> {
    let source = format!(
        "# Summary\n\n{}",
        "A short paragraph with an example and practical details.\n\n".repeat(500)
    );
    let preset = ContentPreset::from_toml(
        r#"
version = 1
name = "bench"
[[gates]]
id = "length"
kind = "word_count"
min = 10
[[gates]]
id = "terms"
kind = "forbidden_terms"
terms = ["forbidden", "exaggerated"]
[[gates]]
id = "heading"
kind = "required_headings"
headings = ["Summary"]
"#,
    )?;
    let input = markdown_input("benchmark.md", &source)?;
    let mut eval = Vec::new();
    let mut parse_and_eval = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        black_box(preset.evaluate(black_box(&input))?);
        eval.push(start.elapsed().as_micros());
        let start = Instant::now();
        let parsed = markdown_input("benchmark.md", black_box(&source))?;
        black_box(preset.evaluate(&parsed)?);
        parse_and_eval.push(start.elapsed().as_micros());
    }
    eval.sort_unstable();
    parse_and_eval.sort_unstable();
    println!("input_bytes={} iterations=100", source.len());
    println!("evaluation_us p50={} p95={}", eval[50], eval[95]);
    println!(
        "markdown_and_evaluation_us p50={} p95={}",
        parse_and_eval[50], parse_and_eval[95]
    );
    Ok(())
}
