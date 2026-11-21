//! Synthetic, in-process incremental versus full content evaluation benchmark.
//! Run: cargo run --release -p aexeo-core --example incremental_content_bench -- 20
use aexeo_contracts::content_gate_protocol::{
    Base, BaseKind, Change, ContentState, Entry, Operation, Origin, OriginKind,
};
use aexeo_core::{
    content_candidate::{CandidateItem, CandidateSnapshot, overlay_candidate},
    content_gate::{ContentInput, ContentPreset, ContentReport},
    content_incremental::{IncrementalBaseline, IncrementalEvaluation},
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, hint::black_box, time::Instant};

fn state(item: &CandidateItem) -> ContentState {
    ContentState {
        revision: item.revision.clone(),
        sha256: item.sha256.clone(),
        inline: Some(item.source.clone()),
        file: None,
        uri: None,
    }
}

fn full(snapshot: &CandidateSnapshot, preset: &ContentPreset) -> BTreeMap<String, ContentReport> {
    snapshot
        .items
        .iter()
        .map(|item| {
            let input = ContentInput {
                id: item.entity_id.clone(),
                text: Some(item.source.clone()),
                ..Default::default()
            };
            (
                item.entity_id.clone(),
                preset.evaluate(&input).expect("fixture is valid"),
            )
        })
        .collect()
}

fn main() -> anyhow::Result<()> {
    let iterations = std::env::args()
        .nth(1)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(20)
        .max(1);
    let source = "A practical sentence with enough detail to review. ".repeat(30);
    let replacement = format!("{source} Updated content.");
    let base = Base {
        kind: BaseKind::Opaque,
        revision: "benchmark".into(),
    };
    let items: Vec<_> = (0..500)
        .map(|i| CandidateItem {
            entity_id: format!("page-{i:04}"),
            public_path: format!("/page-{i:04}"),
            media_type: "text/plain".into(),
            revision: Some("r1".into()),
            sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
            source: source.clone(),
            links: None,
        })
        .collect();
    let after: Vec<_> = items
        .iter()
        .take(10)
        .map(|item| CandidateItem {
            revision: Some("r2".into()),
            sha256: format!("{:x}", Sha256::digest(replacement.as_bytes())),
            source: replacement.clone(),
            ..item.clone()
        })
        .collect();
    let entries = items
        .iter()
        .zip(&after)
        .map(|(old, new)| Entry {
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
    let preset = ContentPreset::from_toml(
        "version=1\nname='bench'\n[[gates]]\nid='words'\nkind='word_count'\nmin=10\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['forbidden']\n",
    )?;
    let prepare_start = Instant::now();
    let baseline = IncrementalBaseline::prepare(snapshot.clone(), preset.clone())
        .map_err(|e| anyhow::anyhow!(e.reason))?;
    let prepare_us = prepare_start.elapsed().as_micros();
    let candidate = overlay_candidate(&snapshot, &change, &after)
        .map_err(anyhow::Error::msg)?
        .candidate;
    let IncrementalEvaluation::Evaluated {
        reports,
        evaluated,
        reused,
        ..
    } = baseline.evaluate(&change, &after)
    else {
        anyhow::bail!("incremental fixture not checked")
    };
    anyhow::ensure!(evaluated == 10 && reused == 490, "unexpected reuse counts");
    anyhow::ensure!(
        serde_json::to_value(&reports)? == serde_json::to_value(full(&candidate, &preset))?,
        "full and incremental results differ"
    );
    let checksum = format!("{:x}", Sha256::digest(serde_json::to_vec(&reports)?));
    let mut incremental = Vec::with_capacity(iterations);
    let mut fresh = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        black_box(baseline.evaluate(black_box(&change), black_box(&after)));
        incremental.push(start.elapsed().as_micros());
        let start = Instant::now();
        let candidate =
            overlay_candidate(black_box(&snapshot), black_box(&change), black_box(&after))
                .map_err(anyhow::Error::msg)?
                .candidate;
        black_box(full(&candidate, black_box(&preset)));
        fresh.push(start.elapsed().as_micros());
    }
    incremental.sort_unstable();
    fresh.sort_unstable();
    let percentile = |samples: &[u128], p: usize| {
        samples[(samples.len() * p).div_ceil(100).max(1) - 1] as f64 / 1000.0
    };
    println!(
        "{}",
        serde_json::json!({"name":"incremental_content/500_plain_items_10_updates", "items":500,"updates":10,"source_bytes_per_item":source.len(),"iterations":iterations,"evaluated":evaluated,"reused":reused,"prepare_ms":prepare_us as f64/1000.0,"incremental_p50_ms":percentile(&incremental,50),"incremental_p95_ms":percentile(&incremental,95),"full_p50_ms":percentile(&fresh,50),"full_p95_ms":percentile(&fresh,95),"result_sha256":checksum})
    );
    Ok(())
}
