//! Compare composed literal-gate evaluation on a reproducible synthetic input.
use aexeo_core::content_gate::{ContentInput, ContentPreset, GateStatus};
use std::{hint::black_box, time::Instant};

fn main() -> anyhow::Result<()> {
    let source = "Alpha café and beta are repeated in this synthetic paragraph.\n\n".repeat(2_000);
    let input = ContentInput {
        id: "synthetic".into(),
        text: Some(source.clone()),
        ..Default::default()
    };
    let mut config = String::from("version=1\nname='literal-benchmark'\n");
    for index in 0..12 {
        config.push_str(&format!("[[gates]]\nid='literal-{index}'\nkind='required_terms'\nterms=['alpha', 'beta']\ncase_insensitive=true\n"));
    }
    let preset = ContentPreset::from_toml(&config)?;
    let report = preset.evaluate(&input)?;
    anyhow::ensure!(
        report.gates.len() == 12
            && report
                .gates
                .iter()
                .all(|gate| gate.status == GateStatus::Passed && gate.findings.is_empty()),
        "unexpected fixture findings"
    );
    if std::env::args().any(|arg| arg == "--report") {
        println!("{}", serde_json::to_string(&report)?);
    }
    let mut samples = Vec::with_capacity(30);
    for _ in 0..30 {
        let start = Instant::now();
        black_box(preset.evaluate(black_box(&input))?);
        samples.push(start.elapsed().as_micros());
    }
    samples.sort_unstable();
    if std::env::args().any(|arg| arg == "--json") {
        let avg_ms = samples.iter().sum::<u128>() as f64 / samples.len() as f64 / 1_000.0;
        println!(
            "{}",
            serde_json::json!({
                "name": "content/composed_literal_gates",
                "avg_ms": avg_ms, "p50_ms": samples[14] as f64 / 1_000.0,
                "p95_ms": samples[28] as f64 / 1_000.0, "iterations": samples.len(),
                "input_bytes": source.len(), "gates": 12
            })
        );
    } else {
        eprintln!(
            "{}",
            serde_json::json!({
                "event": "synthetic_composed_literal_benchmark", "version": 1,
                "input_bytes": source.len(), "gates": 12, "iterations": samples.len(),
                "p50_us": samples[14], "p95_us": samples[28]
            })
        );
    }
    Ok(())
}
