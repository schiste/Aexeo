use seogeo_core::config::Config;
use seogeo_core::static_check::run_native_static_audit_with_config;
use std::fs;
use std::time::{Duration, Instant};

fn write(path: &std::path::Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

fn fixture_root(page_count: usize) -> tempfile::TempDir {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();
    write(
        &root.join("index.html"),
        "<html lang=\"en\"><head><title>Home</title><meta name=\"description\" content=\"Root\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>Home</h1><section data-ui=\"hero\"><h2>Overview</h2><p>Root overview text long enough to satisfy structural thresholds.</p></section></body></html>",
    );
    write(
        &root.join("llms.txt"),
        "# Example\n\n## Pages\n- [Home](/)\n",
    );
    write(
        &root.join("robots.txt"),
        "User-agent: *\nAllow: /\nSitemap: https://example.com/sitemap.xml\n",
    );
    let mut sitemap_entries = vec!["<url><loc>https://example.com/</loc></url>".to_string()];
    for index in 0..page_count {
        let route = format!("guides/page-{index}");
        write(
            &root.join(format!("{route}/index.html")),
            &format!(
                "<html lang=\"en\"><head><title>Guide {index}</title><meta name=\"description\" content=\"Guide {index}\"><link rel=\"canonical\" href=\"https://example.com/{route}\"></head><body><h1>Guide {index}</h1><section data-ui=\"answer\"><h2>Answer</h2><p>Guide body {index} with enough text to count as a real answer block for the benchmark fixture.</p></section><script type=\"application/ld+json\">{{\"@context\":\"https://schema.org\",\"@type\":\"WebPage\",\"name\":\"Guide {index}\"}}</script></body></html>"
            ),
        );
        sitemap_entries.push(format!("<url><loc>https://example.com/{route}</loc></url>"));
    }
    write(
        &root.join("sitemap.xml"),
        &format!("<urlset>{}</urlset>", sitemap_entries.join("")),
    );
    temp_dir
}

fn benchmark_once(config: &Config) -> Duration {
    let fixture = fixture_root(75);
    let started = Instant::now();
    let _ = run_native_static_audit_with_config(fixture.path(), config).unwrap();
    started.elapsed()
}

fn main() {
    let iterations = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10);
    let config = Config::default();
    let mut total = Duration::ZERO;
    for _ in 0..iterations {
        total += benchmark_once(&config);
    }
    println!(
        "static_audit/full_fixture_75_pages: iterations={} total_ms={} avg_ms={}",
        iterations,
        total.as_millis(),
        total.as_millis() / iterations as u128
    );
}
