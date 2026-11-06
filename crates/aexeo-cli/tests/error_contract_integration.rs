//! The failure half of the CLI contract in `SPEC.md`: which exit code each kind
//! of failure gets, what it says, and on which stream.

mod support;

use std::path::Path;
use std::process::{Command, Output};

use support::{bin, parse_json, write};

fn run(args: &[&str]) -> Output {
    Command::new(bin()).args(args).output().unwrap()
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

const MISSING: &str = "/nonexistent/aexeo-error-contract";

fn site() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    write(
        &temp.path().join("index.html"),
        "<html><head><title>Home</title></head><body><h1>Home</h1></body></html>",
    );
    temp
}

// --- exit 2: the command could not be carried out as asked ---------------

#[test]
fn a_nonexistent_site_path_exits_2_and_names_the_path() {
    let output = run(&["check", MISSING]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains(MISSING), "{}", stderr(&output));
}

#[test]
fn an_unsupported_config_version_exits_2() {
    let temp = site();
    write(&temp.path().join("aexeo.toml"), "version = 99\n");
    let output = run(&["check", &temp.path().to_string_lossy()]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("unsupported config version 99"),
        "{}",
        stderr(&output)
    );
}

/// Every loader of external input has to agree, whichever command reached it;
/// each of these used to exit 1 or 70 depending on the command.
#[test]
fn load_failures_exit_2_from_every_command_family() {
    let cases: &[&[&str]] = &[
        &["intelligence", "score", MISSING],
        &["intelligence", "evidence", "assess", MISSING],
        &["intelligence", "grounding-map", MISSING],
        &["intelligence", "facts", "validate", MISSING],
        &["intelligence", "identity", "/", "--path", MISSING],
        &["bing-ai", "import", MISSING],
        &["indexnow", "ledger", "--root", MISSING],
        &["diff", MISSING, MISSING],
        &["report", "render", MISSING],
        &["link-graph", MISSING],
        &["content", MISSING, "--preset", "article"],
        &["perf", "diff", MISSING, MISSING],
    ];
    for args in cases {
        let output = run(args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: stdout {} stderr {}",
            String::from_utf8_lossy(&output.stdout),
            stderr(&output)
        );
    }
}

/// `intelligence score` split one failure across two codes: exit 1 for a site
/// it could not load, exit 70 for a manifest it could not parse.
#[test]
fn intelligence_score_treats_a_bad_site_and_a_bad_manifest_alike() {
    let temp = site();
    write(&temp.path().join("facts.json"), "{ not json");
    let bad_manifest = run(&["intelligence", "score", &temp.path().to_string_lossy()]);
    let bad_site = run(&["intelligence", "score", MISSING]);
    assert_eq!(bad_manifest.status.code(), Some(2));
    assert_eq!(bad_site.status.code(), Some(2));
    assert!(
        stderr(&bad_manifest).contains("facts.json"),
        "{}",
        stderr(&bad_manifest)
    );
}

#[test]
fn a_missing_subcommand_exits_2() {
    assert_eq!(run(&[]).status.code(), Some(2));
    assert_eq!(run(&["bing-ai"]).status.code(), Some(2));
}

#[test]
fn flag_combinations_clap_cannot_express_exit_2() {
    let temp = site();
    let output = run(&[
        "check",
        &temp.path().to_string_lossy(),
        "--regressions-only",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

#[test]
fn an_unreachable_crawl_target_exits_2_from_every_runtime_command() {
    let temp = tempfile::tempdir().unwrap();
    let dead = "http://127.0.0.1:9/";
    for args in [
        vec!["crawl", dead],
        vec!["profile", "runtime", dead],
        vec!["perf", "baseline", dead],
        vec!["content", "--crawl-url", dead, "--preset", "seo-website"],
    ] {
        let output = run_in(temp.path(), &args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            stderr(&output)
        );
    }
}

// --- the cause chain ------------------------------------------------------

#[test]
fn a_config_error_surfaces_the_offending_field() {
    let temp = site();
    write(
        &temp.path().join("aexeo.toml"),
        "version = 1\n[site]\nsource_dir = 42\n",
    );
    let output = run(&["check", &temp.path().to_string_lossy()]);
    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(
        message.contains("failed to deserialize merged config"),
        "{message}"
    );
    assert!(
        message.contains("source_dir"),
        "the field error must survive: {message}"
    );
}

#[test]
fn a_preset_error_surfaces_the_parse_failure() {
    let temp = tempfile::tempdir().unwrap();
    let preset = temp.path().join("repro.toml");
    let content = temp.path().join("page.md");
    write(
        &preset,
        "version = 1\nname = 'repro'\n[[gates]]\nid = 'x'\nkind = 'nope'\n",
    );
    write(&content, "# Title\n");
    let output = run(&[
        "content",
        &content.to_string_lossy(),
        "--preset",
        "repro",
        "--preset-source",
        &format!("repro={}", preset.display()),
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let message = stderr(&output);
    assert!(message.contains("invalid preset repro"), "{message}");
    assert!(
        message.contains("nope"),
        "the parse failure must survive: {message}"
    );
}

// --- streams ---------------------------------------------------------------

/// JSON callers parse stdout, and stderr may carry one telemetry event, so a
/// failure is a JSON document on stdout and no plain text on stderr.
#[test]
fn json_failures_are_structured_on_stdout_with_the_cause_chain() {
    let temp = site();
    write(
        &temp.path().join("aexeo.toml"),
        "version = 1\n[site]\nsource_dir = 42\n",
    );
    let output = run(&["check", &temp.path().to_string_lossy(), "--format", "json"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
    let failure = parse_json(&output.stdout);
    assert_eq!(failure["command"], "check");
    assert_eq!(failure["success"], false);
    assert!(
        failure["error"].as_str().unwrap().contains("source_dir"),
        "{failure}"
    );
}

#[test]
fn json_failures_name_the_full_subcommand() {
    let output = run(&["perf", "diff", MISSING, MISSING, "--format", "json"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(parse_json(&output.stdout)["command"], "perf diff");
}

/// A consumer of `--format sarif` reads a SARIF document or nothing.
#[test]
fn non_json_failures_leave_stdout_empty() {
    for format in ["text", "sarif"] {
        let output = run(&["check", MISSING, "--format", format]);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty(), "{format}");
        assert!(!output.stderr.is_empty(), "{format}");
    }
}

// --- warnings -------------------------------------------------------------

#[test]
fn text_fix_emits_each_config_warning_once() {
    let temp = site();
    write(
        &temp.path().join("aexeo.toml"),
        "site_url = \"https://example.com\"\n",
    );
    let output = run(&["fix", &temp.path().to_string_lossy()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let message = stderr(&output);
    assert_eq!(
        message.matches("site_url").count(),
        1,
        "expected a single warning: {message}"
    );
}

// --- report location ------------------------------------------------------

/// `.aexeo-reports` inside build output would be deployed with the site.
#[test]
fn reports_for_a_source_dir_go_to_the_project_not_into_the_build_output() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    let dist = project.join("dist");
    write(
        &project.join("aexeo.toml"),
        "version = 1\n[site]\nsource_dir = \"dist\"\n",
    );
    write(
        &dist.join("index.html"),
        "<html><head><title>Home</title></head><body><h1>Home</h1></body></html>",
    );

    for args in [
        vec!["intelligence", "score"],
        vec!["intelligence", "evidence", "assess"],
        vec!["check"],
    ] {
        let mut args = args;
        let target = dist.to_string_lossy().to_string();
        args.push(&target);
        let output = run(&args);
        assert!(
            matches!(output.status.code(), Some(0 | 1)),
            "{args:?}: {}",
            stderr(&output)
        );
    }

    assert!(
        !dist.join(".aexeo-reports").exists(),
        "reports were published with the site"
    );
    let reports = project.join(".aexeo-reports");
    assert!(reports.join("intelligence-score-latest.json").is_file());
    assert!(reports.join("evidence-latest.json").is_file());
    assert!(reports.join("check-latest.json").is_file());
}

/// The trend reader has to find what the `check` writer wrote.
#[test]
fn trend_reads_the_reports_check_wrote_for_a_source_dir() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    let dist = project.join("dist");
    write(
        &project.join("aexeo.toml"),
        "version = 1\n[site]\nsource_dir = \"dist\"\n",
    );
    write(
        &dist.join("index.html"),
        "<html><head><title>H</title></head><body></body></html>",
    );
    let target = dist.to_string_lossy().to_string();
    run(&["check", &target]);
    let output = run(&["trend", "check", &target]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("No trend history"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn a_site_with_no_project_above_it_keeps_reports_in_its_root() {
    let temp = site();
    let output = run(&["intelligence", "score", &temp.path().to_string_lossy()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(temp.path().join(".aexeo-reports").is_dir());
}

// --- content coordinates --------------------------------------------------

/// `file:line:col` must be a real position in that file; the projected prose
/// offsets stay available only under their own explicit label.
#[test]
fn content_text_prints_source_coordinates_in_file_position_form() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("article.md");
    let preset = temp.path().join("preset.toml");
    write(
        &file,
        "---\ntitle: hidden\n---\n# Héllo *world*\n\nEscaped \\*star\\*.\n",
    );
    write(
        &preset,
        "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['world','star']\n",
    );
    let path = file.to_string_lossy().to_string();
    let source = format!("custom={}", preset.display());
    let args = [
        "content",
        &path,
        "--input-format",
        "markdown",
        "--preset",
        "custom",
        "--preset-source",
        &source,
    ];

    let text = String::from_utf8_lossy(&run(&args).stdout).to_string();
    assert!(text.contains(&format!("{path}:4:10: ")), "{text}");
    assert!(
        !text.contains("Source location"),
        "the position belongs on the finding: {text}"
    );
    assert!(
        !text.contains(&format!("{path}:1:")),
        "projected coordinates presented as source coordinates: {text}"
    );
    assert!(
        text.contains("(projected text "),
        "an unmapped finding is labelled: {text}"
    );

    // JSON keeps both coordinate systems exactly as before.
    let mut json_args = args.to_vec();
    json_args.extend(["--format", "json"]);
    let report = parse_json(&run(&json_args).stdout);
    assert_eq!(report["gates"][0]["findings"][0]["line"], 1);
    assert_eq!(report["source_locations"][0]["line"], 4);
    assert_eq!(report["source_locations"][0]["column"], 10);
}
