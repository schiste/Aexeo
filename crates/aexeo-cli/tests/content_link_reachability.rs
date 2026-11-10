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
