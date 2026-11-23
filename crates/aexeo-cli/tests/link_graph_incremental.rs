use std::{fs, process::Command};

fn fixture() -> serde_json::Value {
    serde_json::json!({
        "version":1,
        "identity":{"origin":"https://example.org", "release":"r1", "surface":"public", "snapshot":"s1"},
        "snapshot_date":"2026-11-23", "crawl_complete":false, "inventory":null,
        "pages":[
            {"url":"https://example.org/a", "html":"<main><a href='/b'>B</a></main>", "status_code":200,"response_headers":{}},
            {"url":"https://example.org/b", "html":"<main>B</main>", "status_code":200,"response_headers":{}}
        ]
    })
}

#[test]
fn batch_candidates_match_full_reports_and_expose_coverage_counters() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.json");
    let one = dir.path().join("one.json");
    let two = dir.path().join("two.json");
    let input = fixture();
    fs::write(&base, serde_json::to_vec(&input).unwrap()).unwrap();
    let mut changed = input.clone();
    changed["pages"][0]["html"] = "<main><a href='/new'>new</a></main>".into();
    fs::write(&one, serde_json::to_vec(&changed).unwrap()).unwrap();
    fs::write(&two, serde_json::to_vec(&input).unwrap()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph-incremental")
        .arg("--base")
        .arg(&base)
        .arg(&one)
        .arg(&two)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let lines: Vec<serde_json::Value> = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["pages_reused"], 1);
    assert_eq!(lines[0]["pages_extracted"], 1);
    assert_eq!(lines[1]["pages_reused"], 2);
    assert_eq!(lines[1]["destinations_recounted"], 0);
    for (line, path) in lines.iter().zip([one, two]) {
        let full = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .arg("link-graph")
            .arg(path)
            .output()
            .unwrap();
        assert!(full.status.success());
        assert_eq!(
            line["report"],
            serde_json::from_slice::<serde_json::Value>(&full.stdout).unwrap()
        );
    }
}

#[test]
fn ambiguous_json_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.json");
    let bad = dir.path().join("bad.json");
    fs::write(&base, serde_json::to_vec(&fixture()).unwrap()).unwrap();
    fs::write(&bad, b"{\"version\":1,\"version\":1}").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph-incremental")
        .arg("--base")
        .arg(&base)
        .arg(&bad)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    let partial = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph-incremental")
        .arg("--base")
        .arg(&base)
        .arg(&base)
        .arg(&bad)
        .output()
        .unwrap();
    assert!(!partial.status.success());
    let lines = String::from_utf8(partial.stdout).unwrap();
    assert_eq!(lines.lines().count(), 1);
    assert!(serde_json::from_str::<serde_json::Value>(lines.trim()).is_ok());
}
