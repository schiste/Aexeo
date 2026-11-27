use aexeo_core::content_gate::ContentGate;
use aexeo_core::content_presets::{builtin_content_presets, resolve_content_preset};

#[test]
fn news_and_opinion_compose_distinct_advisory_rhythm_reviews() {
    let sources = builtin_content_presets();
    for locale in ["en", "fr"] {
        for (kind, window, deviation, maximum) in [("news", 6, 6, 2), ("opinion", 8, 5, 3)] {
            let profile = resolve_content_preset(&format!("{kind}-style-{locale}"), &sources)
                .expect("built-in profile must resolve");
            assert!(profile.gates.iter().any(|gate| matches!(gate,
                ContentGate::SentenceLengthVariance { window_size, min_stddev_words: Some(minimum), blocking: false, .. }
                    if *window_size == window && *minimum == deviation)));
            assert!(profile.gates.iter().any(|gate| matches!(gate,
                ContentGate::SentenceOpeningWindow { window_size, max_occurrences, blocking: false, .. }
                    if *window_size == window && *max_occurrences == maximum)));
        }
    }
}

#[test]
fn profiles_report_real_passage_and_withhold_short_variance_window() {
    use aexeo_core::content_gate::{ContentContext, ContentInput, ContentParagraph, GateStatus};
    let text = "Team writes news. Team checks news. Team reads news. Team edits news. Team shares news. Team updates news.";
    let input = ContentInput {
        text: Some(text.into()),
        ..Default::default()
    };
    let context = ContentContext {
        paragraphs: Some(vec![ContentParagraph {
            text: text.into(),
            line: 1,
            column: 1,
        }]),
        ..Default::default()
    };
    let sources = builtin_content_presets();
    for (profile, variance_status) in [
        ("news-style-en", GateStatus::Failed),
        ("opinion-style-en", GateStatus::NotChecked),
    ] {
        let preset = resolve_content_preset(profile, &sources).unwrap();
        let result = preset
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        let gate = |id: &str| result.gates.iter().find(|gate| gate.gate_id == id).unwrap();
        assert_eq!(gate("editorial.sentence_variance").status, variance_status);
        assert_eq!(
            gate("editorial.sentence_opening_window").status,
            GateStatus::Failed
        );
        assert!(!gate("editorial.sentence_opening_window").blocking);
    }
}

#[test]
fn structural_and_french_register_presets_compose_without_cross_locale_terms() {
    use aexeo_core::content_gate::{ContentContext, ContentInput, ContentParagraph, GateStatus};
    let sources = builtin_content_presets();
    let structural = resolve_content_preset("style-structure-review", &sources).unwrap();
    let french = resolve_content_preset("style-register-review-fr", &sources).unwrap();
    for id in [
        "editorial.sentence_rhythm",
        "editorial.sentence_variance",
        "editorial.sentence_opening_window",
    ] {
        assert!(structural.gates.iter().any(|gate| gate.id() == id));
        assert!(french.gates.iter().any(|gate| gate.id() == id));
    }
    assert!(
        !structural
            .gates
            .iter()
            .any(|gate| gate.id() == "editorial.fr_phrasing_review")
    );
    let text = "Ils y apportent leur expérience de la technologie, des données, de la sécurité et de la conformité, ainsi que leur connaissance des difficultés que rencontrent les petites équipes.\n\nLeur ambition est simple : que la taille de votre organisation ne vous empêche pas de prendre soin de sa sécurité.";
    let input = ContentInput {
        text: Some(text.into()),
        ..Default::default()
    };
    let context = ContentContext {
        paragraphs: Some(vec![ContentParagraph {
            text: text.into(),
            line: 1,
            column: 1,
        }]),
        ..Default::default()
    };
    let report = french
        .evaluate_with_context(&input, None, None, Some(&context))
        .unwrap();
    let gate = report
        .gates
        .iter()
        .find(|gate| gate.gate_id == "editorial.fr_phrasing_review")
        .unwrap();
    assert_eq!(gate.status, GateStatus::Failed);
    assert!(!gate.blocking);
    assert_eq!(gate.findings.len(), 3);
    assert!(
        gate.findings
            .iter()
            .all(|finding| finding.message.starts_with("Phrase to review in context:"))
    );
    assert!(gate.findings.iter().all(|finding| finding.suggestion.as_deref() == Some("Review this phrase in context; keep it when it fits the meaning and intended voice.")));
    let enumeration = report
        .gates
        .iter()
        .find(|gate| gate.gate_id == "editorial.fr_long_enumeration_review")
        .unwrap();
    assert_eq!(enumeration.status, GateStatus::Failed);
    assert!(!enumeration.blocking);
    assert_eq!(enumeration.measurements["candidate_enumerations"], 1);
    let no_text = french.evaluate(&ContentInput::default()).unwrap();
    let gate = no_text
        .gates
        .iter()
        .find(|gate| gate.gate_id == "editorial.fr_phrasing_review")
        .unwrap();
    assert_eq!(gate.status, GateStatus::NotChecked);
}

#[test]
fn register_terms_can_be_overridden_per_publication() {
    use aexeo_core::content_gate::ContentGate;
    let mut sources = builtin_content_presets();
    sources.insert(
        "publisher-fr".into(),
        r#"version = 1
name = "Publisher French style"
extends = ["style-register-review-fr"]
[[gates]]
id = "editorial.fr_phrasing_review"
kind = "forbidden_terms"
terms = ["expression propre à la marque"]
case_insensitive = true
blocking = false
"#
        .into(),
    );
    let preset = resolve_content_preset("publisher-fr", &sources).unwrap();
    let gate = preset
        .gates
        .iter()
        .find(|gate| gate.id() == "editorial.fr_phrasing_review")
        .unwrap();
    assert!(
        matches!(gate, ContentGate::ForbiddenTerms { terms, blocking: false, .. } if terms == &["expression propre à la marque"])
    );
}

#[test]
fn review_only_terms_are_nonblocking_and_legacy_wording_is_preserved() {
    use aexeo_core::content_gate::{ContentInput, ContentPreset};
    let invalid = "version=1\nname='invalid'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['sample']\nreview_only=true\nblocking=true\n";
    assert!(ContentPreset::from_toml(invalid).is_err());
    let legacy = "version=1\nname='legacy'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['sample']\n";
    let preset = ContentPreset::from_toml(legacy).unwrap();
    let serialized = serde_json::to_value(&preset.gates[0]).unwrap();
    assert!(serialized.get("review_only").is_none());
    let report = preset
        .evaluate(&ContentInput {
            text: Some("sample".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        report.gates[0].findings[0].message,
        "Forbidden term: sample"
    );
}
