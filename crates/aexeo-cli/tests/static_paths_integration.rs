mod support;

use std::process::{Command, Output};

use support::{bin, parse_json, write};

fn run(args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("the aexeo-cli binary runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("the process exited normally")
}

fn site_fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    write(
        &temp.path().join("index.html"),
        r#"<html><head><title>Aexeo | SEO audits</title><meta name="description" content="Aexeo audits sites for search and answer engines."></head><body><h1>Aexeo</h1><p>Body copy.</p></body></html>"#,
    );
    write(
        &temp.path().join("about.html"),
        r#"<html><head><title>About</title></head><body><h1>About</h1><p>Who we are.</p></body></html>"#,
    );
    temp
}

// --- text output path -------------------------------------------------
//
// `static_commands_integration.rs` exercises `generate` only in JSON mode, so
// the text branch and the file-writing branch were never run.

#[test]
fn generate_llms_text_output_is_the_artifact_itself() {
    let temp = site_fixture();
    let output = run(&["generate", "llms", &temp.path().to_string_lossy()]);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    // In text mode the artifact is printed verbatim, so it starts with a
    // markdown heading and sections its pages, rather than with a summary
    // line the way the bundle kinds do.
    assert!(text.trim_start().starts_with("# "), "{text}");
    assert!(text.contains("## Pages"), "{text}");
    assert!(text.contains("- [Home](/)"), "{text}");
}

#[test]
fn generate_writes_nothing_without_a_write_dir() {
    let temp = site_fixture();
    let root = temp.path();
    let before: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .filter(|name| !name.starts_with("._"))
        .collect();

    let output = run(&["generate", "llms", &root.to_string_lossy()]);
    assert!(output.status.success(), "{}", stderr(&output));

    let after: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .filter(|name| !name.starts_with("._"))
        .collect();
    assert_eq!(
        before, after,
        "generate must only print unless --write-dir is given"
    );
}

// --- the machine-artifact branch --------------------------------------
//
// `public-bundle` and `markdown-pages` are the two kinds that go down a
// different path: they build a bundle, write files, and report through
// `machine_bundle_text`. That whole function was untested.

#[test]
fn generate_public_bundle_reports_what_it_wrote() {
    let temp = site_fixture();
    let out = tempfile::tempdir().unwrap();
    let output = run(&[
        "generate",
        "public-bundle",
        &temp.path().to_string_lossy(),
        "--site-url",
        "https://example.com",
        "--write-dir",
        &out.path().to_string_lossy(),
    ]);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Generated"), "{text}");
    assert!(text.contains("Artifacts:"), "{text}");
    assert!(text.contains("Written files:"), "{text}");
    assert!(text.contains("Written paths:"), "{text}");
    assert!(text.contains("llms.txt"), "{text}");

    assert!(
        out.path().join("llms.txt").exists(),
        "the bundle must actually write llms.txt"
    );
    assert!(out.path().join("robots.txt").exists(), "robots.txt missing");
    assert!(
        out.path().join("sitemap.xml").exists(),
        "sitemap.xml missing"
    );
}

#[test]
fn generate_public_bundle_writes_the_discovery_manifest() {
    let temp = site_fixture();
    let out = tempfile::tempdir().unwrap();
    let output = run(&[
        "generate",
        "public-bundle",
        &temp.path().to_string_lossy(),
        "--site-url",
        "https://example.com",
        "--write-dir",
        &out.path().to_string_lossy(),
    ]);

    assert!(output.status.success(), "{}", stderr(&output));
    let manifest = out.path().join("manifest.json");
    assert!(manifest.exists(), "expected a discovery manifest");
    let manifest_bytes = std::fs::read(&manifest).expect("manifest is readable");
    let payload = parse_json(&manifest_bytes);
    assert!(
        payload["artifacts"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "the manifest must list the artifacts it wrote: {payload}"
    );
    // Every artifact the manifest lists must actually be on disk, or the
    // manifest is a promise the bundle does not keep.
    for artifact in payload["artifacts"].as_array().unwrap() {
        let path = artifact["path"].as_str().expect("artifact path");
        assert!(
            out.path().join(path).exists(),
            "{path} listed but not written"
        );
    }
}

// --- guards that exit 2 ----------------------------------------------
//
// `robots`, `sitemap` and `schema` all require a site URL, and a sitemap over
// a site with no indexable routes is refused rather than emitted empty. These
// return EXIT_UNSUPPORTED (2), which is a distinct contract from findings (1)
// and success (0) — and none of it was tested.

#[test]
fn sitemap_without_a_site_url_is_refused_rather_than_guessed() {
    let temp = site_fixture();
    let output = run(&["generate", "sitemap", &temp.path().to_string_lossy()]);

    assert_eq!(code(&output), 2, "expected EXIT_UNSUPPORTED");
    assert!(
        stdout(&output).contains("site_url is required"),
        "the message must say what is missing: {}",
        stdout(&output)
    );
}

#[test]
fn robots_without_a_site_url_is_refused() {
    let temp = site_fixture();
    let output = run(&["generate", "robots", &temp.path().to_string_lossy()]);
    assert_eq!(code(&output), 2, "expected EXIT_UNSUPPORTED");
    assert!(stdout(&output).contains("site_url is required"));
}

/// An empty `<urlset/>` is what a crawler sees when every page is `noindex`
/// or the path was wrong. Publishing it is worse than publishing nothing, so
/// the command refuses and says what to check.
#[test]
fn an_entirely_noindex_site_produces_no_sitemap() {
    let temp = tempfile::tempdir().unwrap();
    write(
        &temp.path().join("index.html"),
        r#"<html><head><title>Hidden</title><meta name="robots" content="noindex"></head><body><h1>Hidden</h1></body></html>"#,
    );
    let output = run(&[
        "generate",
        "sitemap",
        &temp.path().to_string_lossy(),
        "--site-url",
        "https://example.com",
    ]);

    assert_eq!(code(&output), 2, "expected EXIT_UNSUPPORTED");
    let text = stdout(&output);
    assert!(text.contains("no indexable routes"), "{text}");
    assert!(
        !text.contains("<urlset"),
        "an empty sitemap must never be emitted: {text}"
    );
}

/// With a site URL the locations must be absolute. A relative `<loc>` is
/// invalid per the sitemaps protocol and crawlers reject the whole file.
#[test]
fn sitemap_with_a_site_url_emits_absolute_locations() {
    let temp = site_fixture();
    let output = run(&[
        "generate",
        "sitemap",
        &temp.path().to_string_lossy(),
        "--site-url",
        "https://example.com",
    ]);

    assert!(output.status.success(), "{}", stderr(&output));
    let xml = stdout(&output);
    assert!(xml.contains("<loc>https://example.com/</loc>"), "{xml}");
    assert!(!xml.contains("<loc>/"), "a relative location: {xml}");
}

// --- argument validation ---------------------------------------------

#[test]
fn an_unknown_generate_kind_is_rejected() {
    let temp = site_fixture();
    let output = run(&["generate", "not-a-kind", &temp.path().to_string_lossy()]);
    assert!(!output.status.success(), "expected a usage failure");
    assert!(!stderr(&output).is_empty());
}

/// `--site-url` is how a one-off invocation overrides the configured value, so
/// the flag must win over `aexeo.toml`.
#[test]
fn the_site_url_flag_overrides_the_configured_value() {
    let temp = site_fixture();
    write(
        &temp.path().join("aexeo.toml"),
        "version = 1\n\n[site]\nurl = \"https://configured.example\"\n",
    );
    let output = run(&[
        "generate",
        "sitemap",
        &temp.path().to_string_lossy(),
        "--site-url",
        "https://override.example",
    ]);

    assert!(output.status.success(), "{}", stderr(&output));
    let xml = stdout(&output);
    assert!(xml.contains("https://override.example/"), "{xml}");
    assert!(
        !xml.contains("configured.example"),
        "the flag must win over the config: {xml}"
    );
}
