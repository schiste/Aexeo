//! Synthetic removed-route reference benchmark; fixture creation is excluded.
//! Run with `cargo run --release -p aexeo-core --example removed_route_bench -- 5`.
use aexeo_contracts::content_gate_protocol::{Base, BaseKind};
use aexeo_core::content_candidate::{
    CandidateItem, CandidateSnapshot, compare_removed_route_references,
};
use sha2::{Digest, Sha256};
use std::{hint::black_box, time::Instant};

fn main() {
    let iterations = std::env::args()
        .nth(1)
        .map(|s| s.parse::<usize>().expect("iteration count"))
        .unwrap_or(5)
        .max(1);
    let hash = format!("{:x}", Sha256::digest(b"x"));
    let routes: Vec<String> = (0..40).map(|i| format!("/removed-{i:02}")).collect();
    let item = |id: String, path: String, links: Vec<String>| CandidateItem {
        entity_id: id,
        public_path: path,
        media_type: "text/plain".into(),
        revision: None,
        sha256: hash.clone(),
        source: "x".into(),
        links: Some(links),
    };
    let base = CandidateSnapshot {
        scope: "published".into(),
        site_id: "benchmark".into(),
        site_base_url: None,
        base: Base {
            kind: BaseKind::Opaque,
            revision: "benchmark".into(),
        },
        complete: true,
        items: routes
            .iter()
            .enumerate()
            .map(|(i, path)| item(format!("old-{i:02}"), path.clone(), vec![]))
            .collect(),
    };
    let candidate = CandidateSnapshot {
        items: (0..20_000)
            .map(|i| {
                item(
                    format!("source-{i:05}"),
                    format!("/source-{i:05}"),
                    routes.clone(),
                )
            })
            .collect(),
        ..base.clone()
    };
    // Input setup and result serialization are outside the timed section.
    let expected = compare_removed_route_references(&base, &candidate).expect("valid fixture");
    assert!(!expected.complete);
    assert_eq!(expected.references.len(), 10_000);
    let checksum = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&expected).unwrap())
    );
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        let result = black_box(
            compare_removed_route_references(black_box(&base), black_box(&candidate)).unwrap(),
        );
        samples.push(start.elapsed().as_micros());
        assert_eq!(result, expected);
    }
    samples.sort_unstable();
    let percentile =
        |p: usize| samples[(samples.len() * p).div_ceil(100).max(1) - 1] as f64 / 1000.0;
    println!(
        "{}",
        serde_json::json!({"name":"removed_routes/20000_sources_800000_links", "sources":20000, "links":800000, "iterations":iterations, "p50_ms":percentile(50), "p95_ms":percentile(95), "result_sha256":checksum})
    );
}
