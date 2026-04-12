use anyhow::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::route_from_urlish;

pub(super) fn read_sitemap_routes(path: &Path, root: &Path) -> (BTreeSet<String>, Option<String>) {
    if !path.exists() {
        return (BTreeSet::new(), None);
    }
    let mut seen = BTreeSet::new();
    match read_sitemap_routes_recursive(path, root, &mut seen) {
        Ok(routes) => (routes, None),
        Err(error) => (BTreeSet::new(), Some(error.to_string())),
    }
}

pub(super) fn read_sitemap_routes_from_text(text: &str) -> Result<BTreeSet<String>> {
    if text.contains("<sitemapindex") {
        return Ok(BTreeSet::new());
    }
    if !text.contains("<urlset") {
        anyhow::bail!("invalid sitemap XML")
    }
    Ok(read_loc_values(text)
        .into_iter()
        .filter_map(|value| route_from_urlish(&value))
        .collect())
}

fn read_loc_values(text: &str) -> Vec<String> {
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

fn read_sitemap_routes_recursive(
    path: &Path,
    root: &Path,
    seen: &mut BTreeSet<PathBuf>,
) -> Result<BTreeSet<String>> {
    if !seen.insert(path.to_path_buf()) {
        return Ok(BTreeSet::new());
    }
    let text = fs::read_to_string(path)?;
    if text.contains("<sitemapindex") {
        let mut routes = BTreeSet::new();
        for value in read_loc_values(&text) {
            let nested_name = value
                .split('?')
                .next()
                .unwrap_or(&value)
                .split('#')
                .next()
                .unwrap_or(&value)
                .rsplit('/')
                .next()
                .unwrap_or(&value);
            let nested = root.join(nested_name);
            if nested.exists() {
                routes.extend(read_sitemap_routes_recursive(&nested, root, seen)?);
            }
        }
        return Ok(routes);
    }
    read_sitemap_routes_from_text(&text)
}
