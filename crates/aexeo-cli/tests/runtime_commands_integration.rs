mod runtime_support;
mod support;

use aexeo_contracts::{AuditArtifact, AuditPerformance, CrawlStats, PhaseTiming, RuleTiming};
use runtime_support::{spawn_server, spawn_server_with_explicit_shutdown};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;
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
        .arg(".aexeo-reports/crawl-latest.json")
        .arg("--format")
        .arg("text")
        .output()
        .unwrap();
    assert!(
        verify_output.status.success(),
        "verify failed: status={} stdout={} stderr={}",
        verify_output.status,
        String::from_utf8_lossy(&verify_output.stdout),
        String::from_utf8_lossy(&verify_output.stderr)
    );
    let verify_stdout = String::from_utf8_lossy(&verify_output.stdout);
    assert!(verify_stdout.contains("New findings: 0"));
    handle.join().unwrap();
}

#[test]
fn crawl_uses_configured_runtime_engine_when_flag_is_omitted() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("aexeo.toml"),
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
            "AEXEO_PLAYWRIGHT_EXECUTABLE",
            "/definitely/missing/aexeo-playwright-runner",
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
        .env("AEXEO_PLAYWRIGHT_EXECUTABLE", &runner)
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
    // `code()` is Some for any normal exit, so assert the documented
    // success codes rather than mere termination. 1 is "findings", which
    // is still a successful run.
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "expected exit 0 or 1, got {:?}",
        output.status.code()
    );
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "crawl");
    assert!(payload["artifact"]["crawl"]["engine"].as_str().is_some());
    assert!(payload.get("link_graph_summary").is_none());
    assert!(payload.get("error").is_none());
    handle.join().unwrap();
}

#[test]
fn crawl_sarif_emits_config_warnings_on_stderr() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("aexeo.toml"),
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
    let artifact_path = crawl_dir.path().join(".aexeo-reports/crawl-latest.json");
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
        .arg(".aexeo-reports/crawl-latest.json")
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

fn assert_http_test_server_responds(base_url: &str) {
    let address = base_url.strip_prefix("http://").unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "GET / HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
}

#[test]
fn explicit_test_server_remains_available_after_idle_interval() {
    let server = spawn_server_with_explicit_shutdown();
    assert_http_test_server_responds(server.base_url());
    // Give the former 150 ms idle cutoff ample room to expire before reconnecting.
    thread::sleep(Duration::from_secs(1));
    assert_http_test_server_responds(server.base_url());
    server.shutdown();
}

#[test]
fn test_server_waits_for_fragmented_request_headers() {
    let server = spawn_server_with_explicit_shutdown();
    let address = server.base_url().strip_prefix("http://").unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    stream.write_all(b"GET /about HTTP/1.1\r\n").unwrap();
    let mut byte = [0_u8; 1];
    let error = stream.read(&mut byte).unwrap_err();
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    stream
        .write_all(b"Host: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("About page"), "{response}");
    server.shutdown();
}

#[test]
fn crawl_checkpoint_and_resume_complete_a_partial_runtime_audit() {
    let server = spawn_server_with_explicit_shutdown();
    let base_url = server.base_url().to_string();
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
    assert!(
        matches!(partial_output.status.code(), Some(0 | 1)),
        "expected exit 0 or 1, got {:?}",
        partial_output.status.code()
    );
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
    let resumed_stdout = String::from_utf8_lossy(&resumed_output.stdout);
    let resumed_stderr = String::from_utf8_lossy(&resumed_output.stderr);
    let resumed_context = format!(
        "exit code: {:?}\nstdout:\n{}\nstderr:\n{}",
        resumed_output.status.code(),
        resumed_stdout,
        resumed_stderr
    );
    assert!(
        matches!(resumed_output.status.code(), Some(0 | 1)),
        "expected exit code 0 or 1\n{resumed_context}"
    );
    let resumed_payload = serde_json::from_slice::<serde_json::Value>(&resumed_output.stdout)
        .unwrap_or_else(|error| {
            panic!("failed to parse resumed payload: {error}\n{resumed_context}")
        });
    assert_eq!(
        resumed_payload["artifact"]["status"], "complete",
        "{resumed_context}"
    );
    let visited_pages = resumed_payload["artifact"]["crawl"]["visited_pages"].as_u64();
    assert!(
        visited_pages.unwrap_or_default() >= 2,
        "expected at least two visited pages, got {visited_pages:?}\n{resumed_context}"
    );
    server.shutdown();
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
    // `code()` is Some for any normal exit, so assert the documented
    // success codes rather than mere termination. 1 is "findings", which
    // is still a successful run.
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "expected exit 0 or 1, got {:?}",
        output.status.code()
    );
    let payload = parse_json(&output.stdout);
    assert!(payload["artifact"]["performance"]["phases"].is_array());
    let progress_artifact = temp_dir
        .path()
        .join(".aexeo-reports")
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
        .join(".aexeo-reports")
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
        .join(".aexeo-reports")
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
        .arg(".aexeo-reports/quality-latest.json")
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
    // This used to assert only that the exit code was `Some(0 | 1)` — which
    // permits both success and failure — and then that four JSON fields had
    // the right *type* without checking any value. Both halves could pass
    // against output that was wrong.
    //
    // `doctor runtime` is environment-dependent: with Playwright installed
    // it reports `available: true` and exits 0; without it, `available:
    // false` and exit 1. The contract worth pinning is that the exit code
    // tracks the reported availability, plus the concrete shape of a
    // non-available report (a diagnostic mode, a named executable, and a
    // non-empty message — otherwise an operator gets nothing to act on).
    let payload = parse_json(&output.stdout);
    let available = payload["available"].as_bool().expect("available is a bool");
    assert_eq!(
        available,
        output.status.success(),
        "exit code and `available` disagree: success={} available={available}; stdout: {}",
        output.status.success(),
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        payload["mode"], "node_module",
        "doctor should report the resolution mode it used"
    );
    assert_eq!(payload["executable"], "node");
    let message = payload["message"].as_str().expect("message is a string");
    if !available {
        assert!(
            !message.trim().is_empty(),
            "an unavailable runtime must explain why; got an empty message"
        );
    } else {
        assert!(
            message.trim().is_empty(),
            "an available runtime should not carry a failure diagnostic; got {message:?}"
        );
    }
}

#[test]
fn content_preset_crawl_preserves_complete_and_partial_status() {
    let server = spawn_server_with_explicit_shutdown();
    let temp = tempfile::tempdir().unwrap();
    for (limit, expected) in [("1", "partial"), ("10", "complete")] {
        let mut command = Command::new(bin());
        command.current_dir(temp.path());
        command.args([
            "content",
            "--crawl-url",
            server.base_url(),
            "--crawl-max-pages",
            limit,
            "--preset",
            "seo-website",
            "--format",
            "json",
        ]);
        if expected == "partial" {
            command.arg("--telemetry");
        }
        let output = command.output().unwrap();
        let report = parse_json(&output.stdout);
        assert_eq!(report["crawl_status"], expected);
        assert_eq!(report["content"]["gates"][0]["gate_id"], "site.seo_geo");
        assert!(report["audit_findings"].is_array());
        if expected == "partial" {
            assert_eq!(report["content"]["gates"][0]["status"], "not_checked");
            assert!(
                report["content"]["gates"][0]["reason"]
                    .as_str()
                    .unwrap()
                    .contains("incomplete")
            );
            let telemetry = parse_json(&output.stderr);
            assert_eq!(telemetry["summary"]["not_checked"], 1);
            let timings = telemetry["gate_timings_ms"].as_array().unwrap();
            assert_eq!(
                timings.len(),
                report["content"]["gates"].as_array().unwrap().len()
            );
            assert!(timings.iter().all(|t| t.as_f64().is_some_and(|n| n >= 0.0)));
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!stderr.contains(server.base_url()));
            assert!(!stderr.contains("site.seo_geo"));
            assert_eq!(output.status.code(), Some(2));
            assert!(report["truncation_reason"].is_string());
        } else {
            assert!(output.stderr.is_empty());
            assert_eq!(output.status.code(), Some(1));
            assert!(report["truncation_reason"].is_null());
        }
    }
    server.shutdown();
}

#[test]
fn crawl_can_write_observational_link_graph_with_private_telemetry() {
    let (base_url, handle) = spawn_server(8);
    let temp_dir = tempfile::tempdir().unwrap();
    let graph_path = temp_dir.path().join("graph.json");
    let output = Command::new(bin())
        .current_dir(temp_dir.path())
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
        .arg("--link-graph-output")
        .arg(&graph_path)
        .arg("--link-graph-telemetry")
        .arg("--link-graph-summary")
        .output()
        .unwrap();
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "crawl");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&graph_path).unwrap()).unwrap();
    assert_eq!(payload["link_graph_summary"]["crawl_complete"], false);
    assert_eq!(
        payload["link_graph_summary"]["page_count"],
        report["pages"].as_array().unwrap().len()
    );
    assert!(
        payload["link_graph_summary"]["issues"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("inventory_missing"))
    );
    assert_eq!(report["crawl_complete"], false);
    assert!(!report["pages"].as_array().unwrap().is_empty());
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("inventory_missing"))
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let telemetry = stderr
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event["event"] == "link_graph_analysis")
        .expect("graph telemetry");
    assert!(telemetry["page_count"].as_u64().unwrap() > 0);
    assert!(!telemetry.to_string().contains(&base_url));
    handle.join().unwrap();
}

#[test]
fn crawl_graph_summary_rejects_sarif_before_fetching() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .current_dir(temp.path())
        .args([
            "crawl",
            "http://127.0.0.1:1",
            "--format",
            "sarif",
            "--link-graph-output",
            "graph.json",
            "--link-graph-summary",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("supported only with text or json"));
    assert!(!temp.path().join("graph.json").exists());
}

#[test]
fn crawl_graph_summary_is_visible_in_text() {
    let (base_url, handle) = spawn_server(8);
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .current_dir(temp.path())
        .args([
            "crawl",
            &base_url,
            "--engine",
            "http",
            "--progress",
            "off",
            "--format",
            "text",
            "--link-graph-output",
            "graph.json",
            "--link-graph-summary",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("Link graph observations:"), "{text}");
    assert!(text.contains("inventory_missing"), "{text}");
    assert!(text.contains("crawl complete: false"), "{text}");
    handle.join().unwrap();
}

/// A local HTTP server that records every request head, so a test can assert
/// what actually crossed the wire. `127.0.0.1` and `localhost` are two distinct
/// hosts to the crawler, which is how these tests get a "third party".
struct RecordingHost {
    port: u16,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl RecordingHost {
    fn start(respond: impl Fn(&str) -> String + Send + 'static) -> Self {
        use std::sync::atomic::Ordering;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (recorded, stopped) = (requests.clone(), stop.clone());
        thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut head = Vec::new();
                let mut buffer = [0_u8; 4096];
                while !head.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(size) if size > 0 => head.extend_from_slice(&buffer[..size]),
                        _ => break,
                    }
                }
                let head = String::from_utf8_lossy(&head).to_lowercase();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                recorded.lock().unwrap().push(head);
                let _ = stream.write_all(respond(&path).as_bytes());
                let _ = stream.flush();
            }
        });
        Self {
            port,
            requests,
            stop,
        }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for RecordingHost {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn http_response(status: &str, extra_headers: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\n{extra_headers}Content-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

const HOME_PAGE: &str = "<html><head><title>Home</title></head><body><h1>Home</h1></body></html>";

/// Crawl `base_url` with a secret in every credential slot and return the
/// parsed JSON output.
fn credentialed_crawl(base_url: &str, dir: &Path) -> serde_json::Value {
    write(
        &dir.join("aexeo.toml"),
        "version = 1\n\n[runtime]\nheaders = { \"x-secret\" = \"header-secret\" }\ncookies = [{ session = \"cookie-secret\" }]\nbasic_auth = { username = \"user\", password = \"pass\" }\n",
    );
    let output = Command::new(bin())
        .current_dir(dir)
        .arg("crawl")
        .arg(base_url)
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("5")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "crawl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output.stdout)
}

fn assert_no_credentials(requests: &[String]) {
    for request in requests {
        for secret in ["x-secret", "cookie", "authorization"] {
            assert!(
                !request.contains(secret),
                "{secret} crossed to another host: {request}"
            );
        }
    }
}

#[test]
fn crawl_does_not_fetch_a_sitemap_on_another_host() {
    let other = RecordingHost::start(|_| {
        http_response(
            "200 OK",
            "",
            "application/xml",
            "<urlset><url><loc>/x</loc></url></urlset>",
        )
    });
    let other_port = other.port;
    let crawl = RecordingHost::start(move |path| match path {
        "/" => http_response("200 OK", "", "text/html", HOME_PAGE),
        "/sitemap.xml" => http_response(
            "200 OK",
            "",
            "application/xml",
            &format!(
                "<sitemapindex><sitemap><loc>http://localhost:{other_port}/stolen.xml</loc></sitemap></sitemapindex>"
            ),
        ),
        _ => http_response("404 Not Found", "", "text/plain", "missing"),
    });
    let temp_dir = tempfile::tempdir().unwrap();

    let payload = credentialed_crawl(&format!("http://127.0.0.1:{}", crawl.port), temp_dir.path());

    assert!(
        other.requests().is_empty(),
        "a sitemap on another host must not be fetched at all: {:?}",
        other.requests()
    );
    assert!(
        crawl.requests()[0].contains("x-secret: header-secret"),
        "the crawl origin still gets its credentials"
    );
    let text = payload.to_string();
    assert!(
        text.contains("refused to fetch sitemap") && text.contains("stolen.xml"),
        "the refusal must be reported, not silently dropped: {text}"
    );
}

#[test]
fn crawl_redirect_to_another_host_does_not_carry_credentials() {
    let other = RecordingHost::start(|_| http_response("200 OK", "", "text/html", HOME_PAGE));
    let location = format!("Location: http://localhost:{}/landing\r\n", other.port);
    let crawl = RecordingHost::start(move |path| match path {
        "/" => http_response("302 Found", &location, "text/plain", ""),
        _ => http_response("404 Not Found", "", "text/plain", "missing"),
    });
    let temp_dir = tempfile::tempdir().unwrap();

    credentialed_crawl(&format!("http://127.0.0.1:{}", crawl.port), temp_dir.path());

    assert!(
        !other.requests().is_empty(),
        "the redirect is still followed"
    );
    assert_no_credentials(&other.requests());
}

/// The reported attack: a 302 to the same host on another port used to rebase
/// the crawl there, sending every later request and its credentials over.
#[test]
fn crawl_redirect_to_another_port_neither_rebases_nor_carries_credentials() {
    let other = RecordingHost::start(|_| http_response("200 OK", "", "text/html", HOME_PAGE));
    let location = format!("Location: http://127.0.0.1:{}/\r\n", other.port);
    let crawl = RecordingHost::start(move |path| match path {
        "/" => http_response("302 Found", &location, "text/plain", ""),
        _ => http_response("404 Not Found", "", "text/plain", "missing"),
    });
    let temp_dir = tempfile::tempdir().unwrap();

    credentialed_crawl(&format!("http://127.0.0.1:{}", crawl.port), temp_dir.path());

    assert_no_credentials(&other.requests());
    assert!(
        other
            .requests()
            .iter()
            .all(|request| request.starts_with("get / ")),
        "only the redirect target may reach the other port: {:?}",
        other.requests()
    );
    assert!(
        crawl
            .requests()
            .iter()
            .any(|request| request.starts_with("get /robots.txt ")),
        "follow-up requests stay on the crawl origin: {:?}",
        crawl.requests()
    );
}

#[test]
fn crawl_artifacts_redact_set_cookie_and_checkpoints_are_private() {
    let crawl = RecordingHost::start(|path| match path {
        "/" | "/next" => http_response(
            "200 OK",
            "Set-Cookie: session=server-secret; Secure\r\n",
            "text/html",
            "<html><head><title>Home</title></head><body><a href=\"/next\">n</a><a href=\"/more\">m</a></body></html>",
        ),
        _ => http_response("404 Not Found", "", "text/plain", "missing"),
    });
    let temp_dir = tempfile::tempdir().unwrap();
    let checkpoint = temp_dir.path().join("crawl.checkpoint.json");
    let output = Command::new(bin())
        .current_dir(temp_dir.path())
        .arg("crawl")
        .arg(format!("http://127.0.0.1:{}", crawl.port))
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("1")
        .arg("--checkpoint")
        .arg(&checkpoint)
        .arg("--checkpoint-every")
        .arg("1")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(matches!(output.status.code(), Some(0 | 1)));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("server-secret"), "{stdout}");

    let saved = fs::read_to_string(&checkpoint).expect("truncated crawl keeps its checkpoint");
    assert!(!saved.contains("server-secret"), "{saved}");
    assert!(saved.contains("[redacted]"), "{saved}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&checkpoint).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

/// Crawl a public IP literal through a local forward proxy that plays the site.
///
/// This is how these tests get a public origin end to end with no test seam in
/// the binary: `1.1.1.1` classifies as public, so the crawl runs with strict
/// egress, while `HTTP_PROXY` keeps every request on this machine. Each hop still
/// goes through the policy before it is handed to the proxy, so a refused address
/// never shows up at the proxy.
///
/// Not exercised: the DNS resolver gate, because a proxy resolves names itself.
/// That path is covered by `a_public_origin_cannot_redirect_to_a_name_resolving_privately`
/// in `runtime/http.rs`. Nothing reaches a real public network.
fn crawl_public_origin_through_proxy(proxy_port: u16, dir: &Path, config: &str) -> Output {
    write(&dir.join("aexeo.toml"), config);
    let proxy = format!("http://127.0.0.1:{proxy_port}");
    Command::new(bin())
        .current_dir(dir)
        .env("HTTP_PROXY", &proxy)
        .env("http_proxy", &proxy)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .arg("crawl")
        .arg("http://1.1.1.1/")
        .arg("--engine")
        .arg("http")
        .arg("--max-pages")
        .arg("3")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap()
}

const CREDENTIALED_CONFIG: &str =
    "version = 1\n\n[runtime]\nheaders = { \"x-secret\" = \"header-secret\" }\n";

/// A proxy for a site whose `/` redirects to the private address.
fn public_site_redirecting_home_to_private() -> RecordingHost {
    RecordingHost::start(|url| match url {
        "http://1.1.1.1/" => http_response(
            "302 Found",
            "Location: http://169.254.169.254/latest/meta-data/?token=hunter2\r\n",
            "text/plain",
            "",
        ),
        _ => http_response("404 Not Found", "", "text/plain", "missing"),
    })
}

fn proxied_to_private_address(proxy: &RecordingHost) -> Vec<String> {
    proxy
        .requests()
        .into_iter()
        .filter(|request| request.contains("169.254.169.254"))
        .collect()
}

#[test]
fn crawl_from_a_public_origin_refuses_a_redirect_to_a_private_address() {
    let proxy = public_site_redirecting_home_to_private();
    let temp_dir = tempfile::tempdir().unwrap();

    let output =
        crawl_public_origin_through_proxy(proxy.port, temp_dir.path(), CREDENTIALED_CONFIG);

    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "the origin answered, so this is a result: {:?} {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        proxy
            .requests()
            .iter()
            .any(|request| request.starts_with("get http://1.1.1.1/ ")),
        "the public origin must be requested: {:?}",
        proxy.requests()
    );
    assert_eq!(
        proxied_to_private_address(&proxy),
        Vec::<String>::new(),
        "the private address must never be contacted"
    );
    let payload = parse_json(&output.stdout);
    let refusal = payload["artifact"]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|finding| {
            finding["rule_id"] == "CRW001"
                && finding["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("non-public")
        })
        .unwrap_or_else(|| panic!("the refusal must be reported: {payload}"));
    assert_eq!(refusal["severity"], "error");
    assert!(
        payload
            .to_string()
            .contains("169.254.169.254/latest/meta-data")
    );
    assert!(
        !payload.to_string().contains("hunter2"),
        "the Location query must not reach the report: {payload}"
    );
    assert_eq!(payload["artifact"]["status"], "partial", "{payload}");
}

/// The control for the test above: without it, a proxy that never saw the private
/// address could just mean the redirect was never followed.
#[test]
fn allow_private_network_opts_out_of_the_refusal() {
    let proxy = public_site_redirecting_home_to_private();
    let temp_dir = tempfile::tempdir().unwrap();

    crawl_public_origin_through_proxy(
        proxy.port,
        temp_dir.path(),
        "version = 1\n\n[runtime]\nallow_private_network = true\n",
    );

    assert!(!proxied_to_private_address(&proxy).is_empty());
}

/// robots.txt is chosen by the server like any page. The refusal must disallow
/// everything per RFC 9309 and must not read as an unreachable target (exit 2):
/// the origin answered.
#[test]
fn robots_txt_redirect_to_a_private_address_is_refused() {
    let proxy = RecordingHost::start(|url| match url {
        "http://1.1.1.1/robots.txt" => http_response(
            "302 Found",
            "Location: http://169.254.169.254/robots.txt\r\n",
            "text/plain",
            "",
        ),
        _ => http_response("200 OK", "", "text/html", HOME_PAGE),
    });
    let temp_dir = tempfile::tempdir().unwrap();

    let output =
        crawl_public_origin_through_proxy(proxy.port, temp_dir.path(), CREDENTIALED_CONFIG);

    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "{:?} {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(proxied_to_private_address(&proxy), Vec::<String>::new());
    assert!(
        !proxy
            .requests()
            .iter()
            .any(|request| request.starts_with("get http://1.1.1.1/ ")),
        "with robots.txt unknown, no page may be requested: {:?}",
        proxy.requests()
    );
    let payload = parse_json(&output.stdout);
    assert!(
        payload["artifact"]["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["rule_id"] == "CRW002"
                && finding["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("non-public")),
        "{payload}"
    );
}

#[test]
fn nested_sitemaps_obey_robots_txt() {
    let own_port = std::sync::Arc::new(std::sync::atomic::AtomicU16::new(0));
    let port = own_port.clone();
    let crawl = RecordingHost::start(move |path| match path {
        "/" => http_response("200 OK", "", "text/html", HOME_PAGE),
        "/robots.txt" => http_response(
            "200 OK",
            "",
            "text/plain",
            "User-agent: *\nDisallow: /private/\n",
        ),
        "/sitemap.xml" => http_response("200 OK", "", "application/xml", &{
            let port = port.load(std::sync::atomic::Ordering::Relaxed);
            format!(
                "<sitemapindex><sitemap><loc>http://127.0.0.1:{port}/private/nested.xml</loc></sitemap><sitemap><loc>http://127.0.0.1:{port}/open/nested.xml</loc></sitemap></sitemapindex>"
            )
        }),
        _ => http_response("404 Not Found", "", "text/plain", "missing"),
    });
    own_port.store(crawl.port, std::sync::atomic::Ordering::Relaxed);
    let temp_dir = tempfile::tempdir().unwrap();

    credentialed_crawl(&format!("http://127.0.0.1:{}", crawl.port), temp_dir.path());

    let requests = crawl.requests();
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("get /open/nested.xml ")),
        "an allowed nested sitemap is still read: {requests:?}"
    );
    assert!(
        !requests
            .iter()
            .any(|request| request.starts_with("get /private/nested.xml ")),
        "{requests:?}"
    );
}
