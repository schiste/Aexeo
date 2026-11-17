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
