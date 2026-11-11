//! In-process editorial checks on a synthetic single-line page.
//! Fixture creation and site loading are excluded from the measurement.
use aexeo_core::config::{Config, EditorialRouteBrief};
use aexeo_core::{run_editorial_rules, site::load_site};
use std::{hint::black_box, time::Instant};

const WARMUP: usize = 3;

fn benchmark(paragraphs: usize, iterations: usize) -> anyhow::Result<serde_json::Value> {
    let dir = tempfile::tempdir()?;
    let mut html = String::from("<html><body><h1>Editorial benchmark</h1>");
    for _ in 0..paragraphs {
        html.push_str("<p>Une phrase synthétique avec un lien <a href='/other'>ici</a>.</p>");
    }
    html.push_str("</body></html>");
    std::fs::write(dir.path().join("index.html"), &html)?;
    let site = load_site(dir.path())?;
    let mut config = Config::default();
    config
        .editorial
        .routes
        .insert("/".into(), EditorialRouteBrief::default());
    let findings = run_editorial_rules(&site, &config);
    assert_eq!(findings.len(), 2, "fixture must exercise editorial checks");
    for _ in 0..WARMUP {
        black_box(run_editorial_rules(black_box(&site), black_box(&config)));
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        black_box(run_editorial_rules(black_box(&site), black_box(&config)));
        samples.push(started.elapsed().as_micros());
    }
    samples.sort_unstable();
    let percentile =
        |p: usize| samples[(samples.len() * p).div_ceil(100).max(1) - 1] as f64 / 1000.0;
    Ok(serde_json::json!({
        "name": format!("editorial/single_line_{paragraphs}_paragraphs"),
        "paragraphs": paragraphs,
        "input_bytes": html.len(),
        "iterations": iterations,
        "warmup_iterations": WARMUP,
        "avg_ms": samples.iter().sum::<u128>() as f64 / iterations as f64 / 1000.0,
        "p50_ms": percentile(50),
        "p95_ms": percentile(95),
    }))
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|arg| arg == "--json");
    let iterations = args
        .iter()
        .find(|arg| *arg != "--json")
        .map(|arg| arg.parse::<usize>())
        .transpose()?
        .unwrap_or(20)
        .max(1);
    for paragraphs in [1000, 10000] {
        let result = benchmark(paragraphs, iterations)?;
        if json {
            println!("{result}");
        } else {
            println!(
                "{}: avg={}ms p50={}ms p95={}ms",
                result["name"], result["avg_ms"], result["p50_ms"], result["p95_ms"]
            );
        }
    }
    Ok(())
}
