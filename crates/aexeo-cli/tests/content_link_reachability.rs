use std::{fs, net::TcpListener, process::Command};

#[test]
fn network_gate_is_opt_in_and_refuses_private_addresses() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    fs::write(
        &preset,
        "version=1\nname='links'\n[[gates]]\nid='external'\nkind='link_reachability'\n",
    )
    .unwrap();
    fs::write(
        &article,
        format!(
            "[private](http://{}/secret)",
            listener.local_addr().unwrap()
        ),
    )
    .unwrap();
    for enable in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"));
        command.args([
            "content",
            article.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--preset",
            "custom",
            "--preset-source",
            &format!("custom={}", preset.display()),
            "--format",
            "json",
            "--telemetry",
        ]);
        if enable {
            command.arg("--check-link-reachability");
        }
        let output = command.output().unwrap();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["gates"][0]["status"], "not_checked");
        assert!(
            report["gates"][0]["findings"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let logs = String::from_utf8_lossy(&output.stderr);
        assert!(!logs.contains("secret"));
        assert_eq!(logs.contains("content_link_reachability"), enable);
        assert!(listener.accept().is_err());
    }
}

#[test]
fn relative_and_non_http_links_do_not_make_external_checks_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    fs::write(
        &preset,
        "version=1\nname='links'\n[[gates]]\nid='external'\nkind='link_reachability'\n",
    )
    .unwrap();
    fs::write(
        &article,
        "[internal](/docs) [anchor](#section) [mail](mailto:me@example.org)",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            article.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--preset",
            "custom",
            "--preset-source",
            &format!("custom={}", preset.display()),
            "--format",
            "json",
            "--check-link-reachability",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "passed");
    assert_eq!(
        report["gates"][0]["measurements"]["checked_destinations"],
        0
    );
    assert_eq!(
        report["gates"][0]["measurements"]["unchecked_destinations"],
        0
    );
}

#[test]
fn host_link_snapshot_is_bound_to_source_and_order_for_markdown_and_html() {
    use ring::digest::{SHA256, digest};
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article");
    let snapshot = dir.path().join("observations.json");
    fs::write(
        &preset,
        "version=1\nname='links'\n[[gates]]\nid='external'\nkind='link_reachability'\n",
    )
    .unwrap();
    for (format, source) in [
        (
            "markdown",
            "[a](https://example.org/a) [b](https://example.org/b)",
        ),
        (
            "html",
            "<p><a href='https://example.org/a'>a</a> <a href='https://example.org/b'>b</a></p>",
        ),
    ] {
        fs::write(&article, source).unwrap();
        let hash: String = digest(&SHA256, source.as_bytes())
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let exported = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args([
                "content",
                article.to_str().unwrap(),
                "--input-format",
                format,
                "--export-link-observations",
            ])
            .output()
            .unwrap();
        assert!(exported.status.success());
        let mut value: serde_json::Value = serde_json::from_slice(&exported.stdout).unwrap();
        assert_eq!(value["source_sha256"], hash);
        assert_eq!(
            value["destinations"],
            serde_json::json!(["https://example.org/a", "https://example.org/b"])
        );
        assert_eq!(
            value["observations"],
            serde_json::json!(["not_checked", "not_checked"])
        );
        value["observations"] = serde_json::json!(["not_found", "not_checked"]);
        let run = || {
            Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
                .args([
                    "content",
                    article.to_str().unwrap(),
                    "--input-format",
                    format,
                    "--preset",
                    "custom",
                    "--preset-source",
                    &format!("custom={}", preset.display()),
                    "--format",
                    "json",
                    "--link-observations",
                    snapshot.to_str().unwrap(),
                    "--telemetry",
                ])
                .output()
                .unwrap()
        };
        fs::write(&snapshot, serde_json::to_vec(&value).unwrap()).unwrap();
        let output = run();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["gates"][0]["status"], "not_checked");
        assert_eq!(report["gates"][0]["findings"].as_array().unwrap().len(), 1);
        let logs = String::from_utf8_lossy(&output.stderr);
        assert!(logs.contains("content_link_snapshot"));
        assert!(!logs.contains("example.org"));
        value["observations"] = serde_json::json!(["reachable", "reachable"]);
        fs::write(&snapshot, serde_json::to_vec(&value).unwrap()).unwrap();
        let complete = run();
        assert!(complete.status.success());
        let report: serde_json::Value = serde_json::from_slice(&complete.stdout).unwrap();
        assert_eq!(report["gates"][0]["status"], "passed");
        value["destinations"] =
            serde_json::json!(["https://example.org/b", "https://example.org/a"]);
        fs::write(&snapshot, serde_json::to_vec(&value).unwrap()).unwrap();
        let rejected = run();
        assert_eq!(rejected.status.code(), Some(2));
        let error_output = String::from_utf8_lossy(&rejected.stdout);
        assert!(error_output.contains("link observation snapshot does not match"));
        assert!(!error_output.contains("example.org"));
        assert!(!String::from_utf8_lossy(&rejected.stderr).contains("example.org"));
        value["destinations"] =
            serde_json::json!(["https://example.org/a", "https://example.org/b"]);
        value["source_sha256"] = "0".repeat(64).into();
        fs::write(&snapshot, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(!run().status.success());
        value["source_sha256"] = hash.into();
        value["observations"] = serde_json::json!(["reachable"]);
        fs::write(&snapshot, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(!run().status.success());
        value["observations"] = serde_json::json!(["not_found", "not_checked"]);
        let valid = serde_json::to_string(&value).unwrap();
        let duplicate = valid.replacen('{', "{\"version\":1,", 1);
        fs::write(&snapshot, duplicate).unwrap();
        assert!(!run().status.success());
        value["unexpected"] = serde_json::json!(true);
        fs::write(&snapshot, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(!run().status.success());
    }
}

#[test]
fn host_snapshot_does_not_connect_to_destinations() {
    use ring::digest::{SHA256, digest};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    let article = dir.path().join("article.md");
    let snapshot = dir.path().join("observations.json");
    let url = format!("http://{}/secret", listener.local_addr().unwrap());
    let source = format!("[private]({url})");
    let hash: String = digest(&SHA256, source.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    fs::write(
        &preset,
        "version=1\nname='links'\n[[gates]]\nid='external'\nkind='link_reachability'\n",
    )
    .unwrap();
    fs::write(&article, source).unwrap();
    fs::write(
        &snapshot,
        serde_json::to_vec(&serde_json::json!({
            "version":1, "source_sha256":hash, "destinations":[url], "observations":["not_found"]
        }))
        .unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            article.to_str().unwrap(),
            "--input-format",
            "markdown",
            "--preset",
            "custom",
            "--preset-source",
            &format!("custom={}", preset.display()),
            "--format",
            "json",
            "--link-observations",
            snapshot.to_str().unwrap(),
            "--telemetry",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["gates"][0]["status"], "failed");
    assert!(listener.accept().is_err());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret"));
}
