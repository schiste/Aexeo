use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn local_stage_timings_are_opt_in_and_private() {
    for enabled in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"));
        command.args([
            "content",
            "-",
            "--preset",
            "article",
            "--input-format",
            "markdown",
            "--format",
            "json",
        ]);
        if enabled {
            command.arg("--telemetry");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"# private title\n\n[private link](https://private.example)")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        if enabled {
            let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
            assert_eq!(event["schema_version"], 1);
            for name in ["input_load_ms", "adapter_ms"] {
                assert!(event["stages"][name].as_f64().unwrap() >= 0.0);
            }
            assert!(event["stages"].get("external_results_load_ms").is_none());
            let text = String::from_utf8(output.stderr).unwrap();
            assert!(!text.contains("private"));
        } else {
            assert!(output.stderr.is_empty());
        }
    }
}

#[cfg(unix)]
#[test]
fn external_result_wait_is_not_counted_as_adapter_time() {
    use std::{
        fs::OpenOptions,
        thread,
        time::{Duration, Instant},
    };
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("results.fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let writer = thread::spawn({
        let fifo = fifo.clone();
        move || {
            let mut file = OpenOptions::new().write(true).open(fifo).unwrap();
            thread::sleep(Duration::from_millis(800));
            file.write_all(b"[]").unwrap();
        }
    });
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            "-",
            "--preset",
            "article",
            "--input-format",
            "markdown",
            "--format",
            "json",
            "--telemetry",
            "--extension-results",
            fifo.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Example content.")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    writer.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    let external = event["stages"]["external_results_load_ms"]
        .as_f64()
        .unwrap();
    let adapter = event["stages"]["adapter_ms"].as_f64().unwrap();
    assert!(external >= 700.0);
    assert!(
        adapter + external <= elapsed + 100.0,
        "overlapping stages: adapter={adapter}, external={external}, total={elapsed}"
    );
}
