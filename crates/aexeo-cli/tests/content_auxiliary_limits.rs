use std::{fs, process::Command};

#[test]
fn oversized_auxiliary_inputs_fail_and_telemetry_is_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.txt");
    let oversized = dir.path().join("private-input");
    fs::write(&input, "article").unwrap();
    fs::File::create(&oversized)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    for (flag, value) in [
        ("--metadata", oversized.display().to_string()),
        (
            "--evidence-source",
            format!("source={}", oversized.display()),
        ),
        ("--preset-source", format!("custom={}", oversized.display())),
    ] {
        for telemetry in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"));
            command.args([
                "content",
                input.to_str().unwrap(),
                "--preset",
                "article",
                flag,
                &value,
            ]);
            if telemetry {
                command.arg("--telemetry");
            }
            let output = command.output().unwrap();
            assert!(!output.status.success(), "{flag}");
            assert!(output.stdout.is_empty());
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(stderr.contains("auxiliary input byte limit"), "{stderr}");
            let event = stderr
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .find(|event| event["event"] == "content_auxiliary_input_limit");
            assert_eq!(event.is_some(), telemetry);
            if let Some(event) = event {
                assert_eq!(event["limit_bytes"], 16 * 1024 * 1024);
                assert!(!event.to_string().contains("private-input"));
            }
        }
    }
}

#[test]
fn oversized_collection_is_rejected_before_json_parsing() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("article.txt");
    let collection = dir.path().join("collection.json");
    fs::write(&input, "article").unwrap();
    fs::File::create(&collection)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            input.to_str().unwrap(),
            "--preset",
            "article",
            "--collection",
            collection.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("collection exceeds the auxiliary input byte limit")
    );
}
