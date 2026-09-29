mod support;

use std::process::{Command, Output};

use support::{bin, fixture, parse_json, write};

fn run(args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("the aexeo-cli binary runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("the process exited normally")
}

/// A minimal but valid site: a titled page with one section, which is enough
/// for every intelligence command to produce a non-empty report.
fn site_fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    write(
        &temp.path().join("index.html"),
        r#"<html><head><title>Aexeo | SEO audits</title><meta name="description" content="Aexeo audits sites for search and answer engines."><script type="application/ld+json">{"@context":"https://schema.org","@type":"Organization","name":"Aexeo","url":"https://aexeo.com"}</script></head><body><h1>Aexeo</h1><section data-ui="hero"><h2>What is Aexeo</h2><p>Aexeo is a linter for search and answer engines.</p><a href="https://example.com/report">benchmark</a></section></body></html>"#,
    );
    write(
        &temp.path().join("facts.json"),
        r#"{"version":1,"organization":{"name":"Aexeo","website":"https://aexeo.com","category":"seo_platform","descriptors":["seo","geo"]}}"#,
    );
    temp
}

// --- text output path -------------------------------------------------
//
// Every other test in this file asserts the JSON contract, so the `_ =>`
// branch in each `command_*` — the text renderer, the report write, and the
// `EXIT_SUCCESS` return — had no coverage at all. The text path is the
// default when `--format` is omitted, so it is what a user actually gets.

#[test]
fn grounding_map_text_output_reports_the_analysis_and_writes_a_report() {
    let temp = site_fixture();
    let root = temp.path();
    let output = run(&["intelligence", "grounding-map", &root.to_string_lossy()]);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Grounding Map"), "{text}");
    assert!(text.contains("Pages analyzed:"), "{text}");
    assert!(text.contains("Top routes:"), "{text}");
    // The report path is echoed so the operator knows where the JSON landed.
    assert!(text.contains("grounding"), "{text}");

    assert!(
        root.join(".aexeo-reports/grounding-map-latest.json")
            .exists(),
        "the text path must still write the machine-readable report"
    );
}

#[test]
fn surfaces_discover_text_output_reports_coverage() {
    let temp = site_fixture();
    let output = run(&[
        "intelligence",
        "surfaces",
        "discover",
        &temp.path().to_string_lossy(),
    ]);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Machine Surface Graph"), "{text}");
    assert!(text.contains("Surfaces discovered:"), "{text}");
    assert!(text.contains("Routes analyzed:"), "{text}");
}

#[test]
fn score_text_output_reports_the_overall_score() {
    let temp = site_fixture();
    let output = run(&["intelligence", "score", &temp.path().to_string_lossy()]);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Intelligence Score"), "{text}");
    assert!(text.contains("Overall score:"), "{text}");
    // With no trust data the optional score must read n/a, not 0.
    assert!(text.contains("External trust alignment: n/a"), "{text}");
}

#[test]
fn evidence_assess_text_output_reports_claims() {
    let temp = site_fixture();
    let output = run(&[
        "intelligence",
        "evidence",
        "assess",
        &temp.path().to_string_lossy(),
    ]);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Evidence Assessment"), "{text}");
    assert!(text.contains("Claims detected:"), "{text}");
}

#[test]
fn facts_generate_text_output_writes_the_manifest() {
    let temp = site_fixture();
    let root = temp.path();
    let output = run(&["intelligence", "facts", "generate", &root.to_string_lossy()]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("Facts"), "{}", stdout(&output));
    assert!(root.join("facts.json").exists());
}

// --- `facts` / `truth` alias -----------------------------------------
//
// `command_intelligence` matches `Some(("facts", m)) | Some(("truth", m))` —
// one handler, two names. An alias that silently drifts is invisible until
// someone depends on the older name, so both are asserted to produce the
// same command identity.

#[test]
fn truth_is_an_alias_for_facts() {
    let temp = site_fixture();
    write(
        &temp.path().join("facts.json"),
        r#"{"version":1,"organization":{"name":"Aexeo","website":"https://aexeo.com","category":"seo_platform","descriptors":["seo"]},"products":[{"name":"Aexeo","category":"software","descriptors":["auditing"]}]}"#,
    );
    let root = temp.path().to_string_lossy().to_string();

    let via_facts = run(&[
        "intelligence",
        "facts",
        "validate",
        &root,
        "--format",
        "json",
    ]);
    let via_truth = run(&[
        "intelligence",
        "truth",
        "validate",
        &root,
        "--format",
        "json",
    ]);

    assert!(via_facts.status.success(), "{}", stderr(&via_facts));
    assert!(via_truth.status.success(), "{}", stderr(&via_truth));
    // The reported command name is the same for both spellings.
    assert_eq!(
        parse_json(&via_facts.stdout)["command"],
        parse_json(&via_truth.stdout)["command"]
    );
    assert_eq!(
        parse_json(&via_truth.stdout)["result"]["validation"]["valid"],
        true
    );
}

// --- failure paths ---------------------------------------------------
//
// `emit_failure` is entirely untested. It has two behaviours worth pinning:
// JSON failures print a structured error on stdout so a machine consumer can
// parse it, while text failures print on stderr so they do not corrupt a
// piped stdout. Both return a non-zero exit.

#[test]
fn a_missing_site_fails_and_explains_why() {
    let missing = tempfile::tempdir().unwrap().path().join("nope");
    let output = run(&[
        "intelligence",
        "evidence",
        "assess",
        &missing.to_string_lossy(),
    ]);

    assert!(!output.status.success(), "a missing input must fail");
    assert!(code(&output) != 0, "expected a non-zero exit");
    assert!(
        !stderr(&output).is_empty(),
        "a failure must say something, not fail silently"
    );
}

#[test]
fn a_json_failure_is_structured_on_stdout_and_stderr_stays_clean() {
    let missing = tempfile::tempdir().unwrap().path().join("nope");
    let output = run(&[
        "intelligence",
        "evidence",
        "assess",
        &missing.to_string_lossy(),
        "--format",
        "json",
    ]);

    assert!(!output.status.success());
    // A machine consumer parses stdout, so the error belongs there in JSON
    // mode — and stderr must not be polluted with the human form.
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "intelligence evidence assess");
    assert_eq!(payload["success"], false);
    assert!(
        !payload["error"].as_str().unwrap_or_default().is_empty(),
        "{payload}"
    );
    assert!(stderr(&output).is_empty(), "{}", stderr(&output));
}

#[test]
fn a_failed_run_writes_no_report_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // A site directory with no HTML at all is not a loadable site.
    let output = run(&[
        "intelligence",
        "evidence",
        "assess",
        &root.to_string_lossy(),
    ]);

    if !output.status.success() {
        assert!(
            !root.join(".aexeo-reports/evidence-latest.json").exists(),
            "a failed run must not leave a report behind that looks valid"
        );
    }
}

// --- dispatch --------------------------------------------------------

#[test]
fn an_unknown_subcommand_is_rejected_rather_than_ignored() {
    let output = run(&["intelligence", "definitely-not-a-command", "."]);
    assert!(!output.status.success(), "an unknown subcommand must fail");
    assert!(!stderr(&output).is_empty());
}

#[test]
fn a_group_subcommand_needs_its_own_subcommand() {
    // `evidence` with no verb is a usage error, not a silent no-op.
    let output = run(&["intelligence", "evidence"]);
    assert!(!output.status.success(), "expected a usage failure");
    assert!(
        stderr(&output).contains("subcommand") || stderr(&output).contains("Usage"),
        "the error should point at the missing verb: {}",
        stderr(&output)
    );
}

#[test]
fn a_bare_intelligence_invocation_is_rejected() {
    let output = run(&["intelligence"]);
    assert!(!output.status.success(), "expected a usage failure");
}

// --- artifact-driven input -------------------------------------------

/// `--from-crawl-artifact` is a different input path from scanning a
/// directory, and it is the one that carries the "partial crawl" warning. A
/// partial input must not be silently treated as a complete one.
#[test]
fn a_partial_crawl_artifact_input_is_flagged_as_partial() {
    let temp = site_fixture();
    let root = temp.path();
    let artifact = root.join(".aexeo-reports/crawl.json");
    write(
        &artifact,
        r#"{"version":1,"command":"crawl","status":"partial","generated_at":1,"summary":{"total":0,"errors":0,"warnings":0,"actionable":0,"heuristic":0},"truncation_reason":"budget exhausted","site":{"root":"/","url":"https://example.com","pages":[],"artifacts":{}}}"#,
    );

    let output = run(&[
        "intelligence",
        "surfaces",
        "discover",
        "--from-crawl-artifact",
        &artifact.to_string_lossy(),
        "--format",
        "json",
    ]);

    // Either the command refuses a partial artifact, or it succeeds and says
    // so. What must not happen is a clean success with no mention of the
    // truncation, which would let an operator trust a partial scan.
    if output.status.success() {
        let payload = parse_json(&output.stdout);
        let serialized = payload.to_string();
        assert!(
            serialized.contains("partial") || serialized.contains("truncat"),
            "a partial input must be visible in the output: {serialized}"
        );
    } else {
        // JSON mode reports failures on stdout so a machine consumer can
        // parse them; stderr stays clean.
        let payload = parse_json(&output.stdout);
        assert_eq!(payload["success"], false, "{payload}");
    }
}

/// A missing artifact file is the most likely operator mistake, and it must
/// name the path it could not read.
#[test]
fn a_missing_crawl_artifact_names_the_path() {
    let missing = tempfile::tempdir().unwrap().path().join("absent.json");
    let output = run(&[
        "intelligence",
        "surfaces",
        "discover",
        "--from-crawl-artifact",
        &missing.to_string_lossy(),
    ]);

    assert!(!output.status.success(), "a missing artifact must fail");
    assert!(
        stderr(&output).contains("absent.json"),
        "the error must name the file: {}",
        stderr(&output)
    );
}

// --- report location -------------------------------------------------

/// Reports are written under `.aexeo-reports` in the site root. A report
/// landing beside `index.html` would be published to production and indexed,
/// so the reports directory is created and the site's own files are left
/// alone.
#[test]
fn reports_go_to_a_reports_directory_and_not_beside_the_published_pages() {
    let temp = site_fixture();
    let root = temp.path();
    let output = run(&["intelligence", "score", &root.to_string_lossy()]);
    assert!(output.status.success(), "{}", stderr(&output));

    let reports = root.join(".aexeo-reports");
    assert!(reports.is_dir(), "expected {}", reports.display());

    let published: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .filter(|name| !name.starts_with("._"))
        .collect();
    assert!(
        published.contains(&"index.html".to_string()),
        "the fixture page must still be where it was: {published:?}"
    );
    assert!(
        !published.iter().any(|name| name.starts_with("report")),
        "a report leaked into the published root: {published:?}"
    );
    assert!(
        !published
            .iter()
            .any(|name| name.ends_with(".json") && name != "facts.json"),
        "unexpected artifact beside the site: {published:?}"
    );
}

#[test]
fn the_trust_surface_fixture_drives_reconcile_and_import() {
    let root = fixture("chau7-mini-site");
    let trust = fixture("chau7-trust-surfaces.json");

    let output = run(&[
        "intelligence",
        "trust-surface",
        "reconcile",
        &trust.to_string_lossy(),
        &root.to_string_lossy(),
        "--site-url",
        "https://chau7.sh",
    ]);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Trust"), "{text}");
}
