use std::path::Path;

use crate::config::RuntimeConfig;

const ASSET_EXTENSIONS: &[&str] = &[
    ".css", ".gif", ".html", ".ico", ".jpeg", ".jpg", ".js", ".json", ".mjs", ".png", ".svg",
    ".txt", ".webp", ".xml",
];

pub(crate) fn response_report_path(route: &str) -> String {
    if route.is_empty() {
        "crawl/index.html".to_string()
    } else {
        format!("crawl/{}/index.html", route)
    }
}

pub(crate) fn should_enqueue_link(target: &str) -> bool {
    let suffix = Path::new(target)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{}", ext.to_ascii_lowercase()))
        .unwrap_or_default();
    suffix.is_empty() || !ASSET_EXTENSIONS.contains(&suffix.as_str()) || suffix == ".html"
}

pub(crate) fn read_loc_values(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut offset = 0;
    while let Some(start_rel) = text[offset..].find("<loc>") {
        let start = offset + start_rel + 5;
        let Some(end_rel) = text[start..].find("</loc>") else {
            break;
        };
        let end = start + end_rel;
        values.push(text[start..end].trim().to_string());
        offset = end + 6;
    }
    values
}

fn route_matches_patterns(route: &str, patterns: &[String]) -> bool {
    patterns.is_empty()
        || patterns
            .iter()
            .any(|pattern| route.contains(pattern.trim_matches('/')))
}

fn route_is_excluded(route: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| route.contains(pattern.trim_matches('/')))
}

pub(crate) fn route_is_allowed(route: &str, runtime: &RuntimeConfig<'_>) -> bool {
    !route.starts_with("cdn-cgi/")
        && !route_is_excluded(route, runtime.crawl_exclude_patterns)
        && route_matches_patterns(route, runtime.crawl_include_patterns)
}

#[cfg(test)]
mod tests {
    use super::route_is_allowed;
    use crate::config::Config;

    #[test]
    fn rejects_cloudflare_infrastructure_routes() {
        let config = Config::default();
        let runtime = config.runtime();
        assert!(!route_is_allowed("cdn-cgi/l/email-protection", &runtime));
        assert!(route_is_allowed("features/terminal-search", &runtime));
    }
}
