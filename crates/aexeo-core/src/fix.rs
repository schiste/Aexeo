use anyhow::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::generate::{build_link_suggestions, render_llms_txt, render_robots_txt};
use crate::site::{Page, Site, load_site};

fn escape_html_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn canonical_url_for_page(route: &str, site_url: &str) -> String {
    let normalized_site = site_url.trim_end_matches('/');
    if route.is_empty() {
        format!("{}/", normalized_site)
    } else {
        format!("{}/{}", normalized_site, route)
    }
}

fn render_missing_head_tags(page: &Page, config: &Config) -> Vec<String> {
    let mut tags = Vec::new();
    if let Some(site_url) = config.site_url.as_deref() {
        if page.canonical.is_none() {
            tags.push(format!(
                "<link rel=\"canonical\" href=\"{}\">",
                escape_html_attr(&canonical_url_for_page(&page.route, site_url))
            ));
        }
        if page.metadata("og:url").is_none() {
            tags.push(format!(
                "<meta property=\"og:url\" content=\"{}\">",
                escape_html_attr(&canonical_url_for_page(&page.route, site_url))
            ));
        }
    }
    if page.metadata("og:title").is_none()
        && let Some(title) = &page.title
    {
        tags.push(format!(
            "<meta property=\"og:title\" content=\"{}\">",
            escape_html_attr(title)
        ));
    }
    if page.metadata("og:description").is_none()
        && let Some(description) = page.meta_description()
    {
        tags.push(format!(
            "<meta property=\"og:description\" content=\"{}\">",
            escape_html_attr(description)
        ));
    }
    if page.metadata("og:type").is_none() {
        tags.push("<meta property=\"og:type\" content=\"website\">".to_string());
    }
    if page.metadata("twitter:card").is_none() && !config.default_twitter_card.is_empty() {
        tags.push(format!(
            "<meta name=\"twitter:card\" content=\"{}\">",
            escape_html_attr(&config.default_twitter_card)
        ));
    }
    tags
}

fn inject_head_tags(raw_text: &str, tags: &[String]) -> String {
    if tags.is_empty() {
        return raw_text.to_string();
    }
    let insertion = format!("  {}\n", tags.join("\n  "));
    if let Some(index) = raw_text.to_ascii_lowercase().find("</head>") {
        let mut updated = String::with_capacity(raw_text.len() + insertion.len());
        updated.push_str(&raw_text[..index]);
        updated.push_str(&insertion);
        updated.push_str(&raw_text[index..]);
        return updated;
    }
    raw_text.to_string()
}

fn apply_html_metadata_fixes(site: &Site, config: &Config) -> Result<Vec<PathBuf>> {
    let mut changed = Vec::new();
    for page in site.route_pages() {
        let raw_text = fs::read_to_string(&page.path)?;
        let updated = inject_head_tags(&raw_text, &render_missing_head_tags(page, config));
        if updated != raw_text {
            fs::write(&page.path, updated)?;
            changed.push(page.path.clone());
        }
    }
    Ok(changed)
}

fn render_related_links_section(
    source_route: &str,
    target_routes: &[String],
    heading: &str,
) -> String {
    let items = target_routes
        .iter()
        .map(|target| {
            let label = target
                .replace('-', " ")
                .replace('/', " / ")
                .split_whitespace()
                .map(|part| {
                    let mut chars = part.chars();
                    match chars.next() {
                        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!("      <li><a href=\"/{}\">{}</a></li>", target, label)
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "\n  <section data-ui=\"related-links\" data-source-route=\"{}\">\n    <h2>{}</h2>\n    <ul>\n{}\n    </ul>\n  </section>\n",
        escape_html_attr(if source_route.is_empty() {
            "/"
        } else {
            source_route
        }),
        escape_html_attr(heading),
        items,
    )
}

fn apply_related_link_insertions(site: &Site, config: &Config) -> Result<Vec<PathBuf>> {
    if !config.enable_link_autofix {
        return Ok(Vec::new());
    }
    let suggestions = build_link_suggestions(site, config.link_suggestion_count);
    let mut changed = Vec::new();
    for (source_route, target_routes) in suggestions {
        let Some(source_page) = site.page(&source_route) else {
            continue;
        };
        let raw_text = fs::read_to_string(&source_page.path)?;
        if raw_text.contains("data-ui=\"related-links\"")
            || raw_text.contains(&config.related_links_heading)
        {
            continue;
        }
        let section = render_related_links_section(
            &source_route,
            &target_routes,
            &config.related_links_heading,
        );
        let updated = if let Some(index) = raw_text.to_ascii_lowercase().find("</body>") {
            let mut text = String::with_capacity(raw_text.len() + section.len());
            text.push_str(&raw_text[..index]);
            text.push_str(&section);
            text.push_str(&raw_text[index..]);
            text
        } else {
            format!("{}{}", raw_text, section)
        };
        if updated != raw_text {
            fs::write(&source_page.path, updated)?;
            changed.push(source_page.path.clone());
        }
    }
    Ok(changed)
}

pub fn apply_safe_fixes(root: &Path, config: &Config) -> Result<Vec<PathBuf>> {
    let mut changed = BTreeSet::new();
    let site = load_site(root)?;

    let llms_path = root.join("llms.txt");
    if site.llms_text.is_some() {
        let generated = render_llms_txt(&site, config.site_url.as_deref());
        if site.llms_text.as_deref() != Some(generated.as_str()) {
            fs::write(&llms_path, generated)?;
            changed.insert(llms_path);
        }
    }

    if let Some(site_url) = config.site_url.as_deref() {
        let robots_path = root.join("robots.txt");
        match site.robots_text.as_deref() {
            None => {
                fs::write(&robots_path, render_robots_txt(site_url))?;
                changed.insert(robots_path);
            }
            Some(text) if !text.to_ascii_lowercase().contains("sitemap:") => {
                let updated = format!(
                    "{}\nSitemap: {}/sitemap.xml\n",
                    text.trim_end(),
                    site_url.trim_end_matches('/')
                );
                fs::write(&robots_path, updated)?;
                changed.insert(robots_path);
            }
            _ => {}
        }
    }

    let refreshed_site = load_site(root)?;
    for path in apply_html_metadata_fixes(&refreshed_site, config)? {
        changed.insert(path);
    }
    let refreshed_site = load_site(root)?;
    for path in apply_related_link_insertions(&refreshed_site, config)? {
        changed.insert(path);
    }

    Ok(changed.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::apply_safe_fixes;
    use crate::config::Config;
    use anyhow::Result;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    fn make_html_page(route: &str, body: &str) -> String {
        let canonical = if route.is_empty() {
            "https://example.com/".to_string()
        } else {
            format!("https://example.com/{}", route)
        };
        format!(
            "<html lang=\"en\"><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"{}\"></head><body><h1>x</h1>{}</body></html>",
            canonical, body,
        )
    }

    #[test]
    fn updates_llms_and_creates_robots_and_head_tags() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("features/alpha/index.html"),
            &make_html_page("features/alpha", ""),
        );
        write(
            &root.join("feature-data.json"),
            r#"{"categories":[{"id":"x","name":"X","features":[{"slug":"alpha"}]}]}"#,
        );
        write(
            &root.join("llms.txt"),
            "# Site\n\n## Key Facts\n- 9 features across 3 categories\n\n## Pages\n- [Home](index.html)\n",
        );
        let config = Config {
            site_url: Some("https://example.com".to_string()),
            ..Config::default()
        };
        let changed = apply_safe_fixes(root, &config)?;
        assert!(!changed.is_empty());
        assert!((root.join("robots.txt")).exists());
        let index_html = fs::read_to_string(root.join("index.html"))?;
        assert!(index_html.contains("rel=\"canonical\""));
        assert!(index_html.contains("property=\"og:title\""));
        assert!(index_html.contains("name=\"twitter:card\""));
        assert!(fs::read_to_string(root.join("llms.txt"))?.contains("features/alpha)"));
        Ok(())
    }

    #[test]
    fn can_insert_related_links_when_enabled() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            &make_html_page("", "<p>Guide to alpha workflows and alpha setup.</p>"),
        );
        write(
            &root.join("guide/index.html"),
            &make_html_page("guide", "<p>Guide body alpha workflows.</p>"),
        );
        write(
            &root.join("alpha/index.html"),
            &make_html_page("alpha", "<p>Alpha workflows reference.</p>"),
        );
        let config = Config {
            site_url: Some("https://example.com".to_string()),
            enable_link_autofix: true,
            related_links_heading: "Related pages".to_string(),
            ..Config::default()
        };
        apply_safe_fixes(root, &config)?;
        let updated = fs::read_to_string(root.join("index.html"))?;
        assert!(updated.contains("/alpha") || updated.contains("data-ui=\"related-links\""));
        Ok(())
    }
}
