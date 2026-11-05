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
            let text = String::from_utf8(output.stderr).unwrap();
            assert!(!text.contains("private"));
        } else {
            assert!(output.stderr.is_empty());
        }
    }
}
