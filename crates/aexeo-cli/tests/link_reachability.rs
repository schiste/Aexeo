use std::{fs, net::TcpListener, process::Command};

#[test]
fn markdown_checks_are_advisory_and_do_not_disclose_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.md");
    fs::write(&input, "[local](http://127.0.0.1/private?secret=123) [again](http://127.0.0.1/private?secret=123#frag) [email](mailto:private@example.org) [relative](/article)").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-reachability")
        .arg(&input)
        .arg("--telemetry")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let events: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(report["observations"][0]["status"], "uncertain");
    assert_eq!(report["observations"][1]["status"], "uncertain");
    assert_eq!(report["observations"][2]["status"], "skipped");
    assert_eq!(report["observations"][3]["status"], "skipped");
    assert_eq!(report["network_checks"], 1);
    assert_eq!(events["event"], "link_reachability");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret"));
}

#[test]
fn adapter_json_is_supported_and_budget_is_validated() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("links.json");
    fs::write(&input, r#"["/relative", "http://127.0.0.1/"]"#).unwrap();
    let run = |limit: &str| {
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .arg("link-reachability")
            .arg(&input)
            .args(["--input-format", "json", "--max-checks", limit])
            .output()
            .unwrap()
    };
    let output = run("1");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["observations"][0]["status"], "skipped");
    assert_eq!(report["observations"][1]["status"], "uncertain");
    assert!(!run("101").status.success());
}

#[test]
fn local_server_receives_no_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("links.json");
    fs::write(
        &input,
        format!("[\"http://{}/private\"]", listener.local_addr().unwrap()),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-reachability")
        .arg(&input)
        .args(["--input-format", "json"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["observations"][0]["status"], "uncertain");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
