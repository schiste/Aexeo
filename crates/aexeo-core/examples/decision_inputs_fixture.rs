//! Synthetic fixture for the independent Python interoperability test.
use aexeo_contracts::content_gate_protocol::Change;
use aexeo_core::content_gate::{ContentInput, ContentPreset};
use aexeo_core::content_gate_sources::ConfinedContentFiles;
use aexeo_core::content_gate_verdict::{
    ChangeCollection, OfflineDecisionContext, evaluate_offline_receipt_with_html_scope,
};
use aexeo_core::html_content_adapter::HtmlContentScope;
use std::{
    io::{self, Cursor},
    path::Path,
};

struct NoFiles;
impl ConfinedContentFiles for NoFiles {
    type Reader = Cursor<Vec<u8>>;
    fn open(&mut self, _: &Path, _: &Path) -> io::Result<Self::Reader> {
        Err(io::Error::other("fixture must not read files"))
    }
}

fn main() -> anyhow::Result<()> {
    let change: Change = serde_json::from_value(serde_json::json!({
        "schema":"content-gate.change/v1", "change_id":"x", "created_at":"2026-01-01T00:00:00Z",
        "origin":{"kind":"manual"}, "base":{"kind":"opaque","revision":"1"},
        "entries":[{"entity_id":"article", "operation":"create", "media_type":"text/plain",
            "after":{"inline":"abc", "sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}}]
    }))?;
    let preset = ContentPreset::from_toml(
        "version=1\nname='fixture'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n",
    )?;
    let items = vec![ContentInput {
        id: "article".into(),
        text: Some("café".into()),
        ..Default::default()
    }];
    let mut fixtures = Vec::new();
    for (present, scope) in [
        (false, HtmlContentScope::Document),
        (true, HtmlContentScope::Document),
        (false, HtmlContentScope::Main),
        (true, HtmlContentScope::Main),
    ] {
        let mut change = change.clone();
        if scope == HtmlContentScope::Main {
            change.entries[0].media_type = "text/html".into();
            let state = change.entries[0]
                .after
                .as_mut()
                .expect("fixture has content");
            state.inline =
                Some("<nav>word</nav><main><p>safe</p></main><footer>word</footer>".into());
            state.sha256 =
                "1a092dfafca8e00d06f070ace94d81061c2749235da07c5aca309f5dc0d106b2".into();
        }
        let context = OfflineDecisionContext {
            change: &change,
            preset: &preset,
            max_entry_bytes: 1024,
            max_total_bytes: 2048,
            plain_text_paragraphs: true,
            collection: present.then_some(ChangeCollection {
                change_id: "x",
                items: &items,
                complete: false,
            }),
            evaluator_identity: b"test-build",
        };
        let receipt = evaluate_offline_receipt_with_html_scope(
            &context,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            scope,
        )?;
        let artifacts = context.artifacts_with_html_scope(scope)?;
        fixtures.push(serde_json::json!({
            "manifest": receipt.decision_inputs,
            "receipt": receipt,
            "artifacts": {"change": artifacts.change, "resolved_policy": artifacts.resolved_policy,
                "host_options": artifacts.host_options, "evaluator": artifacts.evaluator,
                "collection": artifacts.collection, "external_evidence": null}
        }));
    }
    println!("{}", serde_json::to_string(&fixtures)?);
    Ok(())
}
