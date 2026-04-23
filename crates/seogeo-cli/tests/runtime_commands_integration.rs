mod runtime_support;
mod support;

use runtime_support::spawn_server;
use seogeo_contracts::{AuditArtifact, AuditPerformance, CrawlStats, PhaseTiming, RuleTiming};
use std::fs;
use std::path::Path;
use std::process::Command;
use support::{bin, parse_json, write};

#[test]
fn crawl_and_verify_commands_work_end_to_end() {
    let (base_url, handle) = spawn_server(16);
    let temp_dir = tempfile::tempdir().unwrap();
    let crawl_output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert!(!crawl_output.status.success());
    let crawl_stdout = String::from_utf8_lossy(&crawl_output.stdout);
    assert!(crawl_stdout.contains("Audit results:"));

    let verify_output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("verify")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--baseline")
        .arg(".seogeo-reports/crawl-latest.json")
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert!(verify_output.status.success());
    let verify_stdout = String::from_utf8_lossy(&verify_output.stdout);
    assert!(verify_stdout.contains("New findings: 0"));
    handle.join().unwrap();
}

#[test]
fn crawl_uses_configured_runtime_engine_when_flag_is_omitted() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("seogeo.toml"),
        "browser_engine = \"http\"\n",
    );
    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--max-pages")
        .arg("10")
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Audit results:"));
    handle.join().unwrap();
}

#[test]
fn crawl_rejects_playwright_when_runner_override_is_missing() {
    let output = Command::new(bin())
        .env(
            "SEOGEO_PLAYWRIGHT_EXECUTABLE",
            "/definitely/missing/seogeo-playwright-runner",
        )
        .arg("crawl")
        .arg("http://127.0.0.1:1")
        .arg("--engine")
        .arg("playwright")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "crawl");
    assert_eq!(payload["success"], false);
    assert!(
        payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("requires a local Playwright runtime")
            || payload["error"]
                .as_str()
                .unwrap_or_default()
                .contains("does not exist")
    );
}

#[test]
fn crawl_accepts_playwright_with_custom_runner() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    let runner = temp_dir.path().join("playwright-runner.sh");
    write(
        &runner,
        &format!(
            "#!/bin/sh\nprintf '%s' '{{\"statusCode\":200,\"contentType\":\"text/html\",\"body\":\"<html><head><title>Home</title><meta name='\\''description'\\'' content='\\''Root'\\''><link rel='\\''canonical'\\'' href='\\''{base_url}/'\\''></head><body><h1>Home</h1><a href='/about'>About</a></body></html>\",\"headers\":{{\"content-type\":\"text/html\"}},\"effectiveUrl\":\"{base_url}/\"}}'\n"
        ),
    );
    let mut permissions = std::fs::metadata(&runner).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
    }
    std::fs::set_permissions(&runner, permissions).unwrap();

    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .env("SEOGEO_PLAYWRIGHT_EXECUTABLE", &runner)
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("playwright")
        .arg("--max-pages")
        .arg("1")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.code().is_some());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "crawl");
    assert!(payload["artifact"]["crawl"]["engine"].as_str().is_some());
    assert!(payload.get("error").is_none());
    handle.join().unwrap();
}

#[test]
fn crawl_sarif_emits_config_warnings_on_stderr() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("seogeo.toml"),
        "browser_engine = \"http\"\n",
    );
    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--max-pages")
        .arg("10")
        .arg("--format")
        .arg("sarif")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("CFGDEP001"));
    handle.join().unwrap();
}

#[test]
fn crawl_json_contract_reports_summary_and_exit_code() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "crawl");
    assert_eq!(payload["success"], false);
    assert!(
        payload["audit_path"]
            .as_str()
            .unwrap()
            .ends_with("crawl-latest.json")
    );
    assert!(payload["findings"].is_array());
    handle.join().unwrap();
}

#[test]
fn crawl_artifact_can_drive_intelligence_commands_without_scanning_cwd() {
    let (base_url, handle) = spawn_server(8);
    let crawl_dir = tempfile::tempdir().unwrap();
    let crawl_output = Command::new(bin())
        .current_dir(crawl_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--progress")
        .arg("off")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!crawl_output.status.success());
    let crawl_payload = parse_json(&crawl_output.stdout);
    assert_eq!(
        crawl_payload["artifact"]["site"]["pages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let unrelated_dir = tempfile::tempdir().unwrap();
    let artifact_path = crawl_dir.path().join(".seogeo-reports/crawl-latest.json");
    let surfaces_output = Command::new(bin())
        .current_dir(unrelated_dir.path())
        .arg("intelligence")
        .arg("surfaces")
        .arg("discover")
        .arg("--from-crawl-artifact")
        .arg(&artifact_path)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(surfaces_output.status.success());
    let surfaces_payload = parse_json(&surfaces_output.stdout);
    assert_eq!(
        surfaces_payload["result"]["input"]["source"],
        "crawl_artifact"
    );
    assert_eq!(surfaces_payload["result"]["input"]["pages"], 2);
    assert_eq!(
        surfaces_payload["result"]["graph"]["coverage"]["total_routes"],
        2
    );

    let fanout_output = Command::new(bin())
        .current_dir(unrelated_dir.path())
        .arg("intelligence")
        .arg("fanout")
        .arg("assess")
        .arg("--from-crawl-artifact")
        .arg(&artifact_path)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(fanout_output.status.success());
    let fanout_payload = parse_json(&fanout_output.stdout);
    assert_eq!(
        fanout_payload["result"]["input"]["source"],
        "crawl_artifact"
    );
    assert_eq!(fanout_payload["result"]["assessment"]["routes_analyzed"], 2);

    handle.join().unwrap();
}

#[test]
fn verify_json_contract_reports_diff_summary() {
    let (base_url, handle) = spawn_server(16);
    let temp_dir = tempfile::tempdir().unwrap();
    let crawl_output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert!(!crawl_output.status.success());

    let verify_output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("verify")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--baseline")
        .arg(".seogeo-reports/crawl-latest.json")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(verify_output.status.success());
    let payload = parse_json(&verify_output.stdout);
    assert_eq!(payload["command"], "verify");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["summary"]["new"], 0);
    assert!(payload["diff"]["new_findings"].is_array());
    handle.join().unwrap();
}

#[test]
fn crawl_checkpoint_and_resume_complete_a_partial_runtime_audit() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    let checkpoint = temp_dir.path().join("crawl-checkpoint.json");

    let partial_output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("1")
        .arg("--checkpoint")
        .arg(&checkpoint)
        .arg("--checkpoint-every")
        .arg("1")
        .arg("--progress")
        .arg("off")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(partial_output.status.code().is_some());
    assert!(checkpoint.exists());
    let partial_payload = parse_json(&partial_output.stdout);
    assert_eq!(partial_payload["artifact"]["status"], "partial");

    let resumed_output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("10")
        .arg("--resume")
        .arg(&checkpoint)
        .arg("--progress")
        .arg("off")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    let resumed_payload = parse_json(&resumed_output.stdout);
    assert_eq!(resumed_payload["artifact"]["status"], "complete");
    assert!(
        resumed_payload["artifact"]["crawl"]["visited_pages"]
            .as_u64()
            .unwrap()
            >= 2
    );
    handle.join().unwrap();
}

#[test]
fn crawl_writes_progress_artifact_and_performance_metadata() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("5")
        .arg("--artifact-every")
        .arg("1")
        .arg("--partial-audit-every")
        .arg("10")
        .arg("--progress")
        .arg("off")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.code().is_some());
    let payload = parse_json(&output.stdout);
    assert!(payload["artifact"]["performance"]["phases"].is_array());
    let progress_artifact = temp_dir
        .path()
        .join(".seogeo-reports")
        .join("crawl-progress-latest.json");
    assert!(progress_artifact.exists());
    handle.join().unwrap();
}

#[test]
fn profile_runtime_reports_performance_data() {
    let (base_url, handle) = spawn_server(8);
    let output = Command::new(bin())
        .arg("profile")
        .arg("runtime")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("5")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "profile");
    assert!(payload["performance"]["phases"].is_array());
    assert!(payload["crawl"]["elapsed_ms"].as_u64().is_some());
    handle.join().unwrap();
}

#[test]
fn profile_runtime_fails_when_performance_budget_is_exceeded() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    let budget_path = temp_dir.path().join("runtime-budget.json");
    write(&budget_path, r#"{"max_elapsed_ms":0}"#);
    let output = Command::new(bin())
        .arg("profile")
        .arg("runtime")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("5")
        .arg("--perf-budget")
        .arg(&budget_path)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "profile");
    assert_eq!(payload["performance"]["budget"]["passed"], false);
    assert_eq!(
        payload["performance"]["budget"]["violations"][0]["metric"],
        "elapsed"
    );
    handle.join().unwrap();
}

#[test]
fn perf_diff_reports_runtime_regressions() {
    let temp_dir = tempfile::tempdir().unwrap();
    let baseline_path = temp_dir.path().join("baseline.json");
    let current_path = temp_dir.path().join("current.json");
    let baseline = AuditArtifact {
        command: "crawl".to_string(),
        crawl: Some(CrawlStats {
            engine: "http".to_string(),
            visited_pages: 10,
            max_pages: 10,
            elapsed_ms: 100,
            pages_per_minute: 600,
            average_fetch_ms: 20,
            ..CrawlStats::default()
        }),
        performance: Some(AuditPerformance {
            elapsed_us: 100_000,
            wall_clock_us: 100_000,
            cumulative_tracked_us: 50_000,
            phases: vec![PhaseTiming {
                name: "fetch".to_string(),
                elapsed_us: 50_000,
                p95_us: 30_000,
                ..PhaseTiming::default()
            }],
            rule_groups: vec![RuleTiming {
                group: "schema".to_string(),
                elapsed_us: 10_000,
                findings: 1,
            }],
            ..AuditPerformance::default()
        }),
        ..AuditArtifact::default()
    };
    let current = AuditArtifact {
        command: "crawl".to_string(),
        crawl: Some(CrawlStats {
            engine: "http".to_string(),
            visited_pages: 10,
            max_pages: 10,
            elapsed_ms: 140,
            pages_per_minute: 420,
            average_fetch_ms: 40,
            ..CrawlStats::default()
        }),
        performance: Some(AuditPerformance {
            elapsed_us: 140_000,
            wall_clock_us: 140_000,
            cumulative_tracked_us: 80_000,
            phases: vec![PhaseTiming {
                name: "fetch".to_string(),
                elapsed_us: 80_000,
                p95_us: 60_000,
                ..PhaseTiming::default()
            }],
            rule_groups: vec![RuleTiming {
                group: "schema".to_string(),
                elapsed_us: 20_000,
                findings: 1,
            }],
            ..AuditPerformance::default()
        }),
        ..AuditArtifact::default()
    };
    write(&baseline_path, &serde_json::to_string(&baseline).unwrap());
    write(&current_path, &serde_json::to_string(&current).unwrap());

    let output = Command::new(bin())
        .arg("perf")
        .arg("diff")
        .arg(&baseline_path)
        .arg(&current_path)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "perf diff");
    assert_eq!(payload["success"], false);
    assert!(
        payload["report"]["summary"]["regressions"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        payload["report"]["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|metric| metric["metric"] == "phase.fetch.p95_ms" && metric["regressed"] == true)
    );
}

#[test]
fn perf_baseline_writes_named_baseline_and_compares_previous_run() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    let previous_path = temp_dir
        .path()
        .join(".seogeo-reports")
        .join("test-site-runtime-baseline-latest.json");
    let mut previous = AuditArtifact {
        command: "perf-baseline".to_string(),
        crawl: Some(CrawlStats {
            engine: "http".to_string(),
            visited_pages: 2,
            max_pages: 5,
            elapsed_ms: 10_000,
            pages_per_minute: 10,
            average_fetch_ms: 500,
            ..CrawlStats::default()
        }),
        performance: Some(AuditPerformance {
            elapsed_us: 10_000_000,
            wall_clock_us: 10_000_000,
            cumulative_tracked_us: 10_000_000,
            phases: vec![PhaseTiming {
                name: "fetch".to_string(),
                elapsed_us: 5_000_000,
                p95_us: 500_000,
                ..PhaseTiming::default()
            }],
            ..AuditPerformance::default()
        }),
        ..AuditArtifact::default()
    };
    previous.summary.total = 1_000;
    previous.summary.errors = 10;
    write(&previous_path, &serde_json::to_string(&previous).unwrap());

    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("perf")
        .arg("baseline")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("5")
        .arg("--name")
        .arg("Test Site!")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let second_payload = parse_json(&output.stdout);
    assert_eq!(second_payload["command"], "perf baseline");
    assert_eq!(second_payload["name"], "test-site");
    assert_eq!(second_payload["latest_updated"], true);
    let expected_previous_path = previous_path.canonicalize().unwrap();
    assert_eq!(
        second_payload["diff"]["baseline_path"],
        expected_previous_path.display().to_string()
    );
    let latest = second_payload["latest_path"].as_str().unwrap();
    let baseline = second_payload["baseline_path"].as_str().unwrap();
    assert!(Path::new(latest).exists());
    assert!(Path::new(baseline).exists());
    assert_eq!(second_payload["artifact"]["crawl"]["visited_pages"], 2);
    handle.join().unwrap();
}

#[test]
fn perf_baseline_does_not_promote_failed_gate_by_default() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    let previous_path = temp_dir
        .path()
        .join(".seogeo-reports")
        .join("blocked-runtime-baseline-latest.json");
    let previous = AuditArtifact {
        command: "previous-baseline".to_string(),
        generated_at: 1,
        ..AuditArtifact::default()
    };
    write(&previous_path, &serde_json::to_string(&previous).unwrap());
    let budget_path = temp_dir.path().join("runtime-budget.json");
    write(&budget_path, r#"{"max_elapsed_ms":0}"#);

    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("perf")
        .arg("baseline")
        .arg(&base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("5")
        .arg("--name")
        .arg("Blocked")
        .arg("--perf-budget")
        .arg(&budget_path)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["latest_updated"], false);
    assert!(Path::new(payload["baseline_path"].as_str().unwrap()).exists());
    let latest_payload = parse_json(&fs::read(&previous_path).unwrap());
    assert_eq!(latest_payload["command"], "previous-baseline");
    handle.join().unwrap();
}

#[test]
fn report_render_supports_markdown_output() {
    let temp_dir = tempfile::tempdir().unwrap();
    let quality_output = Command::new(bin())
        .arg("quality")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!quality_output.status.success());

    let render_output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("report")
        .arg("render")
        .arg(".seogeo-reports/quality-latest.json")
        .arg("--format")
        .arg("md")
        .output()
        .unwrap();
    assert!(render_output.status.success());
    let stdout = String::from_utf8_lossy(&render_output.stdout);
    assert!(stdout.contains("# Audit Report"));
    assert!(stdout.contains("## Summary"));
}

#[test]
fn runtime_doctor_reports_json_contract() {
    let output = Command::new(bin())
        .arg("doctor")
        .arg("runtime")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(matches!(output.status.code(), Some(0 | 1)));
    let payload = parse_json(&output.stdout);
    assert!(payload["available"].is_boolean());
    assert!(payload["mode"].is_string());
    assert!(payload["message"].is_string());
}
