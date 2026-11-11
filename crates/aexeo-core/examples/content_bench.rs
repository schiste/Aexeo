//! Reproducible content evaluation and adapter benchmarks.
use aexeo_core::{
    content_gate::{ContentEvaluationContext, ContentInput, ContentPreset, MAX_FINDINGS_PER_GATE},
    html_content_adapter::html_content,
    markdown_adapter::{
        markdown_input, markdown_input_with_links, markdown_input_with_source_map,
        markdown_input_with_source_map_and_destinations, markdown_link_destinations,
    },
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
    let html_source = format!(
        "<article><h1>Summary</h1>{}</article>",
        "<p>A short paragraph with <a href=\"https://example.org/path\">an example</a> and practical details.</p>".repeat(500)
    );
    let html_fixture = html_content("benchmark.html", &html_source).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        html_fixture.destinations.len() == 500,
        "HTML benchmark lost links"
    );
    anyhow::ensure!(
        html_fixture
            .context
            .paragraphs
            .as_ref()
            .is_some_and(|paragraphs| paragraphs.len() == 500),
        "HTML benchmark lost paragraphs"
    );
    anyhow::ensure!(
        html_fixture
            .input
            .headings
            .as_ref()
            .is_some_and(|headings| headings.iter().any(|heading| heading == "Summary")),
        "HTML benchmark lost its heading"
    );
    let openings_preset = ContentPreset::from_toml(
        "version=1\nname='openings'\n[[gates]]\nid='opening'\nkind='repeated_paragraph_openings'\nopening_words=3\nmax_occurrences=2\n",
    )?;
    let (openings_input, openings_context, _) = markdown_input_with_links("benchmark.md", &source)?;
    let openings_capabilities = aexeo_core::content_gate::ContentEvaluationContext {
        adapter: Some(&openings_context),
        ..Default::default()
    };
    let input = markdown_input("benchmark.md", &source)?;
    let dense_source = "x ".repeat(200_000);
    let dense_input = ContentInput {
        id: "dense.txt".into(),
        text: Some(dense_source.clone()),
        ..Default::default()
    };
    let dense_preset = ContentPreset::from_toml(
        "version=1\nname='dense'\n[[gates]]\nid='term'\nkind='forbidden_terms'\nterms=['x']\n",
    )?;
    let dense_check = dense_preset.evaluate(&dense_input)?;
    anyhow::ensure!(
        dense_check.gates[0].measurements["matches"] == 200_000
            && dense_check.gates[0].findings.len() == MAX_FINDINGS_PER_GATE,
        "dense benchmark lost matches"
    );
    let mut html_projection = Vec::new();
    let mut html_eval = Vec::new();
    let mut openings_eval = Vec::new();
    let mut eval = Vec::new();
    let mut dense_eval = Vec::new();
    let mut parse_and_eval = Vec::new();
    let mut repeated_eval = Vec::new();
    let mut batch_eval = Vec::new();
    let mut links_eval = Vec::new();
    let mut links_two_parses = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        black_box(
            html_content("benchmark.html", black_box(&html_source)).map_err(anyhow::Error::msg)?,
        );
        html_projection.push(start.elapsed().as_micros());
        let start = Instant::now();
        let html =
            html_content("benchmark.html", black_box(&html_source)).map_err(anyhow::Error::msg)?;
        let html_capabilities = ContentEvaluationContext {
            adapter: Some(&html.context),
            link_count: Some(html.destinations.len()),
            ..Default::default()
        };
        black_box(preset.evaluate_with_capabilities(&html.input, &html_capabilities)?);
        black_box(link_preset.evaluate_with_destinations(
            &html.input,
            &html_capabilities,
            Some(&html.destinations),
        )?);
        html_eval.push(start.elapsed().as_micros());
        let start = Instant::now();
        black_box(
            openings_preset
                .evaluate_with_capabilities(black_box(&openings_input), &openings_capabilities)?,
        );
        openings_eval.push(start.elapsed().as_micros());
        let start = Instant::now();
        black_box(dense_preset.evaluate(black_box(&dense_input))?);
        dense_eval.push(start.elapsed().as_micros());
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
        let adapted =
            markdown_input_with_source_map_and_destinations("links.md", black_box(&link_source))?;
        let input = adapted.input;
        let context = adapted.context;
        let count = adapted.link_count;
        let destinations = adapted.destinations;
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
        let start = Instant::now();
        let destinations = markdown_link_destinations(black_box(&link_source))?;
        let (input, context, count, _map) =
            markdown_input_with_source_map("links.md", black_box(&link_source))?;
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
        links_two_parses.push(start.elapsed().as_micros());
    }
    html_projection.sort_unstable();
    html_eval.sort_unstable();
    openings_eval.sort_unstable();
    eval.sort_unstable();
    dense_eval.sort_unstable();
    parse_and_eval.sort_unstable();
    repeated_eval.sort_unstable();
    batch_eval.sort_unstable();
    links_eval.sort_unstable();
    links_two_parses.sort_unstable();
    if std::env::args().any(|arg| arg == "--json") {
        for (name, samples) in [
            ("content/html_projection", &html_projection),
            ("content/html_projection_and_evaluation", &html_eval),
            ("content/repeated_paragraph_openings", &openings_eval),
            ("content/evaluation", &eval),
            ("content/dense_forbidden_terms", &dense_eval),
            ("content/markdown_and_evaluation", &parse_and_eval),
            ("content/batch_50", &batch_eval),
            ("content/markdown_links_and_evaluation", &links_eval),
            (
                "content/markdown_links_two_parse_baseline",
                &links_two_parses,
            ),
        ] {
            let avg_ms = samples.iter().sum::<u128>() as f64 / samples.len() as f64 / 1000.0;
            println!(
                "{}",
                serde_json::json!({"name": name, "avg_ms": avg_ms, "p50_ms": samples[49] as f64 / 1000.0, "p95_ms": samples[94] as f64 / 1000.0, "iterations": samples.len(), "input_bytes": if name == "content/dense_forbidden_terms" { dense_source.len() } else if name.starts_with("content/html_") { html_source.len() } else if name.starts_with("content/markdown_links_") { link_source.len() } else { source.len() }})
            );
        }
        return Ok(());
    }
    println!("input_bytes={} iterations=100", source.len());
    println!(
        "html_input_bytes={} projection_us p50={} p95={} projection_and_evaluation_us p50={} p95={}",
        html_source.len(),
        html_projection[49],
        html_projection[94],
        html_eval[49],
        html_eval[94]
    );
    println!(
        "paragraph_openings_us p50={} p95={}",
        openings_eval[49], openings_eval[94]
    );
    println!("evaluation_us p50={} p95={}", eval[49], eval[94]);
    println!(
        "dense_forbidden_terms_us p50={} p95={}",
        dense_eval[49], dense_eval[94]
    );
    println!(
        "markdown_links_and_evaluation_us p50={} p95={}",
        links_eval[49], links_eval[94]
    );
    println!(
        "markdown_links_two_parse_baseline_us p50={} p95={}",
        links_two_parses[49], links_two_parses[94]
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
