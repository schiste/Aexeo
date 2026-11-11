use std::{fs, process::Command};

#[test]
fn html_content_uses_composable_gates_and_does_not_guess_source_positions() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    fs::write(
        &input,
        "<h1>Title</h1><p>Intro <a href='http://example.org'>word</a></p><p hidden>secret</p>",
    )
    .unwrap();
    fs::write(&preset, "version=1\nname='test'\n[[gates]]\nid='words'\nkind='forbidden_terms'\nterms=['word','secret']\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\n[[gates]]\nid='online'\nkind='link_reachability'\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--input-format",
            "html",
            "--preset",
            "custom",
            "--preset-source",
            &format!("custom={}", preset.display()),
            "--format",
            "json",
            "--telemetry",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["findings"].as_array().unwrap().len(), 1);
    assert_eq!(report["gates"][1]["status"], "failed");
    assert_eq!(report["gates"][2]["status"], "not_checked");
    assert!(report.get("source_locations").is_none());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("example.org"));
    assert!(!stderr.contains("secret"));
    let event: serde_json::Value = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["event"] == "content_evaluation")
        .unwrap();
    assert_eq!(
        event["html_projection"],
        serde_json::json!({"status":"complete", "headings":1, "paragraphs":1, "links":1})
    );
}

#[test]
fn html_plain_fragment_does_not_claim_source_offsets_even_when_text_is_identical() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.html");
    let preset = dir.path().join("preset.toml");
    fs::write(&input, "word").unwrap();
    fs::write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='words'\nkind='forbidden_terms'\nterms=['word']\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--input-format",
            "html",
            "--preset",
            "custom",
            "--preset-source",
            &format!("custom={}", preset.display()),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report.get("source_locations").is_none());
    assert!(output.stderr.is_empty(), "telemetry must be opt-in");
}

#[test]
fn html_limit_telemetry_reports_unavailable_without_fabricated_counts() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("large.html");
    fs::write(&input, "x".repeat(16 * 1024 * 1024 + 1)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--input-format",
            "html",
            "--preset",
            "article",
            "--format",
            "json",
            "--telemetry",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "not_checked");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let event: serde_json::Value = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["event"] == "content_evaluation")
        .unwrap();
    assert_eq!(
        event["html_projection"],
        serde_json::json!({"status":"unavailable"})
    );
    assert!(!stderr.contains("large.html"));
}
