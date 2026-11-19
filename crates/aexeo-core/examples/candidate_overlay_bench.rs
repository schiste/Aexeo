//! Synthetic candidate overlay benchmark. Fixture creation is excluded.
//! Run with `cargo run --release -p aexeo-core --example candidate_overlay_bench -- 20`.
use aexeo_contracts::content_gate_protocol::{
    Base, BaseKind, Change, ContentState, Entry, Operation, Origin, OriginKind,
};
use aexeo_core::content_candidate::{CandidateItem, CandidateSnapshot, overlay_candidate};
use sha2::{Digest, Sha256};
use std::{hint::black_box, time::Instant};

fn state(item: &CandidateItem) -> ContentState {
    ContentState {
        revision: item.revision.clone(),
        sha256: item.sha256.clone(),
        inline: Some(item.source.clone()),
        file: None,
        uri: None,
    }
}

fn main() {
    let iterations = std::env::args()
        .nth(1)
        .map(|s| s.parse::<usize>().expect("iteration count"))
        .unwrap_or(20)
        .max(1);
    let source = "A".repeat(16 * 1024);
    let replacement_source = "B".repeat(16 * 1024);
    let digest = |s: &str| format!("{:x}", Sha256::digest(s.as_bytes()));
    let old_hash = digest(&source);
    let new_hash = digest(&replacement_source);
    let base = Base {
        kind: BaseKind::Opaque,
        revision: "benchmark".into(),
    };
    let items: Vec<_> = (0..2000)
        .map(|i| CandidateItem {
            entity_id: format!("page-{i:04}"),
            public_path: format!("/page-{i:04}"),
            media_type: "text/plain".into(),
            revision: Some("r1".into()),
            sha256: old_hash.clone(),
            source: source.clone(),
            links: Some(
                (1..=4)
                    .map(|n| format!("/page-{:04}", (i + n) % 2000))
                    .collect(),
            ),
        })
        .collect();
    let after: Vec<_> = items
        .iter()
        .take(100)
        .map(|old| CandidateItem {
            source: replacement_source.clone(),
            sha256: new_hash.clone(),
            revision: Some("r2".into()),
            ..old.clone()
        })
        .collect();
    let entries = after
        .iter()
        .zip(&items)
        .map(|(new, old)| Entry {
            entity_id: new.entity_id.clone(),
            operation: Operation::Update,
            previous_entity_id: None,
            media_type: new.media_type.clone(),
            public_path: Some(new.public_path.clone()),
            metadata: None,
            before: Some(state(old)),
            after: Some(state(new)),
        })
        .collect();
    let snapshot = CandidateSnapshot {
        scope: "published".into(),
        site_id: "benchmark".into(),
        site_base_url: None,
        base: base.clone(),
        complete: true,
        items,
    };
    let change = Change {
        schema: "content-gate.change/v1".into(),
        change_id: "benchmark".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        origin: Origin {
            kind: OriginKind::Manual,
            system: None,
            reference: None,
        },
        base,
        content_root: None,
        site: None,
        author: None,
        entries,
        extensions: None,
    };
    let warmup = overlay_candidate(&snapshot, &change, &after).expect("valid fixture");
    assert_eq!(warmup.candidate.items.len(), 2000);
    assert_eq!(warmup.impact.changed.len(), 100);
    assert!(warmup.impact.link_impact_complete);
    let checksum = format!("{:x}", Sha256::digest(serde_json::to_vec(&warmup).unwrap()));
    for _ in 0..2 {
        black_box(overlay_candidate(&snapshot, &change, &after).unwrap());
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        black_box(
            overlay_candidate(black_box(&snapshot), black_box(&change), black_box(&after)).unwrap(),
        );
        samples.push(start.elapsed().as_micros());
    }
    samples.sort_unstable();
    let percentile =
        |p: usize| samples[(samples.len() * p).div_ceil(100).max(1) - 1] as f64 / 1000.0;
    println!(
        "{}",
        serde_json::json!({"name":"candidate_overlay/2000_pages_100_updates", "pages":2000, "updates":100, "source_bytes":2000*16*1024, "links":8000, "iterations":iterations, "p50_ms":percentile(50), "p95_ms":percentile(95), "result_sha256":checksum})
    );
}
