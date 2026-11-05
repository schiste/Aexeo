//! Reproducible content evaluation and Markdown adapter benchmark.
use aexeo_core::{
    content_gate::ContentPreset,
    markdown_adapter::{markdown_input, markdown_input_with_source_map_and_destinations},
};
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
    let mut repeated_config = String::from("version = 1\nname = \"repeated\"\n");
    for i in 0..20 {
        repeated_config.push_str(&format!(
            "[[gates]]\nid = \"count-{i}\"\nkind = \"word_count\"\nmin = 10\n"
        ));
    }
    let repeated_preset = ContentPreset::from_toml(&repeated_config)?;
    let link_preset = ContentPreset::from_toml(
        "version=1\nname='links'\n[[gates]]\nid='urls'\nkind='link_destinations'\nallowed_schemes=['https']\n",
    )?;
    let link_source = format!(
        "# Links\n\n{}",
        "A [link](https://example.org/path) with text.\n\n".repeat(500)
    );
    let input = markdown_input("benchmark.md", &source)?;
    let mut eval = Vec::new();
    let mut parse_and_eval = Vec::new();
    let mut repeated_eval = Vec::new();
    let mut batch_eval = Vec::new();
    let mut links_eval = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        black_box(preset.evaluate(black_box(&input))?);
        eval.push(start.elapsed().as_micros());
        let start = Instant::now();
        for _ in 0..50 {
            black_box(preset.evaluate(black_box(&input))?);
        }
        batch_eval.push(start.elapsed().as_micros());
        let start = Instant::now();
        black_box(repeated_preset.evaluate(black_box(&input))?);
        repeated_eval.push(start.elapsed().as_micros());
        let start = Instant::now();
        let parsed = markdown_input("benchmark.md", black_box(&source))?;
        black_box(preset.evaluate(&parsed)?);
        parse_and_eval.push(start.elapsed().as_micros());
        let start = Instant::now();
        let (input, context, count, _map, destinations) =
            markdown_input_with_source_map_and_destinations("links.md", black_box(&link_source))?;
        let capabilities = aexeo_core::content_gate::ContentEvaluationContext {
            adapter: Some(&context),
            link_count: Some(count),
            ..Default::default()
        };
        black_box(link_preset.evaluate_with_destinations(
            &input,
            &capabilities,
            destinations.as_deref(),
        )?);
        links_eval.push(start.elapsed().as_micros());
    }
    eval.sort_unstable();
    parse_and_eval.sort_unstable();
    repeated_eval.sort_unstable();
    batch_eval.sort_unstable();
    links_eval.sort_unstable();
    if std::env::args().any(|arg| arg == "--json") {
        for (name, samples) in [
            ("content/evaluation", &eval),
            ("content/markdown_and_evaluation", &parse_and_eval),
            ("content/batch_50", &batch_eval),
            ("content/markdown_links_and_evaluation", &links_eval),
        ] {
            let avg_ms = samples.iter().sum::<u128>() as f64 / samples.len() as f64 / 1000.0;
            println!(
                "{}",
                serde_json::json!({"name": name, "avg_ms": avg_ms, "p50_ms": samples[49] as f64 / 1000.0, "p95_ms": samples[94] as f64 / 1000.0, "iterations": samples.len(), "input_bytes": if name == "content/markdown_links_and_evaluation" { link_source.len() } else { source.len() }})
            );
        }
        return Ok(());
    }
    println!("input_bytes={} iterations=100", source.len());
    println!("evaluation_us p50={} p95={}", eval[49], eval[94]);
    println!(
        "markdown_links_and_evaluation_us p50={} p95={}",
        links_eval[49], links_eval[94]
    );
    println!("batch_50_us p50={} p95={}", batch_eval[49], batch_eval[94]);
    println!(
        "20_word_gates_us p50={} p95={}",
        repeated_eval[49], repeated_eval[94]
    );
    println!(
        "markdown_and_evaluation_us p50={} p95={}",
        parse_and_eval[49], parse_and_eval[94]
    );
    Ok(())
}
