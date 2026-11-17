use std::{
    fs,
    process::{Command, Output},
};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(args)
        .output()
        .unwrap()
}

#[cfg(unix)]
fn run_with_permissive_umask(args: &[&str]) -> Output {
    Command::new("sh")
        .arg("-c")
        .arg("umask 022; exec \"$@\"")
        .arg("sh")
        .arg(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn review_and_explicit_application_are_bound_to_source_and_proposal() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.txt");
    let edits = dir.path().join("edits.json");
    let patch = dir.path().join("patch.json");
    let output = dir.path().join("result.txt");
    fs::write(&source, "One sentence.\n").unwrap();
    fs::write(
        &edits,
        r#"[{"start":0,"end":3,"expected":"One","replacement":"A","reason":"reviewed"}]"#,
    )
    .unwrap();
    let source = source.to_str().unwrap();
    let edits = edits.to_str().unwrap();
    let patch = patch.to_str().unwrap();
    let output = output.to_str().unwrap();
    let proposal = run(&[
        "editorial-patch",
        "propose",
        "--source",
        source,
        "--edits",
        edits,
    ]);
    assert!(
        proposal.status.success(),
        "{}",
        String::from_utf8_lossy(&proposal.stderr)
    );
    fs::write(patch, &proposal.stdout).unwrap();
    let review = run(&[
        "editorial-patch",
        "review",
        "--source",
        source,
        "--patch",
        patch,
        "--telemetry",
    ]);
    assert!(
        review.status.success(),
        "{}",
        String::from_utf8_lossy(&review.stderr)
    );
    assert_eq!(review.stdout, b"A sentence.\n");
    let stderr = String::from_utf8(review.stderr).unwrap();
    let hash = stderr
        .lines()
        .next()
        .unwrap()
        .strip_prefix("reviewed proposal SHA-256: ")
        .unwrap();
    assert!(!stderr.contains("sentence"));
    let args = [
        "editorial-patch",
        "apply",
        "--source",
        source,
        "--patch",
        patch,
        "--output",
        output,
        "--confirm",
        hash,
    ];
    #[cfg(unix)]
    assert!(run_with_permissive_umask(&args).status.success());
    #[cfg(not(unix))]
    assert!(run(&args).status.success());
    assert_eq!(fs::read_to_string(output).unwrap(), "A sentence.\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(output).unwrap().permissions().mode() & 0o077,
            0,
            "candidate must not be readable by other users"
        );
    }
    assert!(!run(&args).status.success());
    fs::remove_file(output).unwrap();
    let original_patch = fs::read(patch).unwrap();
    let mut modified_patch = original_patch.clone();
    modified_patch.push(b' ');
    fs::write(patch, modified_patch).unwrap();
    assert!(!run(&args).status.success());
    assert!(!std::path::Path::new(output).exists());
    fs::write(patch, original_patch).unwrap();
    fs::write(source, "Changed source.\n").unwrap();
    assert!(!run(&args).status.success());
    assert!(!std::path::Path::new(output).exists());
}

#[test]
fn rejects_duplicate_keys() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let edits = dir.path().join("edits");
    fs::write(&source, "a").unwrap();
    fs::write(
        &edits,
        r#"[{"start":0,"start":0,"end":1,"expected":"a","replacement":"b","reason":"x"}]"#,
    )
    .unwrap();
    let result = run(&[
        "editorial-patch",
        "propose",
        "--source",
        source.to_str().unwrap(),
        "--edits",
        edits.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
}

#[test]
fn opt_in_transition_suggestions_stay_reviewable_and_abstain_on_markup() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.txt");
    let patch = dir.path().join("patch.json");
    fs::write(&source, "Moreover, the team acted.\r\n").unwrap();
    let source = source.to_str().unwrap();
    let patch = patch.to_str().unwrap();
    let args = [
        "editorial-patch",
        "suggest-transitions",
        "--source",
        source,
        "--language",
        "en",
        "--input-format",
        "text",
    ];
    let result = run(&args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["status"], "proposed");
    assert_eq!(report["heuristic"], true);
    assert_eq!(
        fs::read_to_string(source).unwrap(),
        "Moreover, the team acted.\r\n"
    );
    fs::write(patch, serde_json::to_vec(&report["patch"]).unwrap()).unwrap();
    let review = run(&[
        "editorial-patch",
        "review",
        "--source",
        source,
        "--patch",
        patch,
    ]);
    assert!(review.status.success());
    assert_eq!(review.stdout, b"The team acted.\r\n");
    assert!(
        !run(&[
            "editorial-patch",
            "suggest-transitions",
            "--source",
            source,
            "--language",
            "en"
        ])
        .status
        .success()
    );
    fs::write(source, "Moreover, the [link](https://example.com) works.").unwrap();
    let blocked = run(&args);
    assert!(blocked.status.success());
    let report: serde_json::Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(report["status"], "abstained");
    assert!(report.get("patch").is_none());
}
