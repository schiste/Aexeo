#[path = "site/parser.rs"]
mod parser;
#[path = "site/sitemap.rs"]
mod sitemap;

use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlternateLink {
    pub href: String,
    pub hreflang: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub href: String,
    pub target: Option<String>,
    pub text: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub tag: String,
    pub data_ui: Option<String>,
    pub line: usize,
    pub column: usize,
    pub has_heading: bool,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailsBlock {
    pub line: usize,
    pub column: usize,
    pub has_summary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreBlock {
    pub line: usize,
    pub column: usize,
    pub has_code: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonLdBlock {
    pub raw: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrawlMeta {
    pub visited_pages: usize,
    pub max_pages: usize,
    pub discovered_internal_routes: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentModel {
    StaticExport,
    SsrWorker,
    RuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PageKind {
    Home,
    NotFound,
    Search,
    Admin,
    Legal,
    Feed,
    Utility,
    Listing,
    Detail,
    Docs,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReference {
    pub src: String,
    pub alt: Option<String>,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub path: PathBuf,
    pub relative_path: String,
    pub route: String,
    pub page_kind: PageKind,
    pub raw_text: String,
    pub title: Option<String>,
    pub meta_by_name: BTreeMap<String, String>,
    pub meta_by_property: BTreeMap<String, String>,
    pub canonical: Option<String>,
    pub html_lang: Option<String>,
    pub h1_count: usize,
    pub h1_texts: Vec<String>,
    pub has_breadcrumb_nav: bool,
    pub response_headers: BTreeMap<String, String>,
    pub links: Vec<Link>,
    pub internal_links: Vec<String>,
    pub alternate_links: Vec<AlternateLink>,
    pub images: Vec<ImageReference>,
    pub blocks: Vec<Block>,
    pub details_blocks: Vec<DetailsBlock>,
    pub pre_blocks: Vec<PreBlock>,
    pub json_ld_blocks: Vec<JsonLdBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    pub root: PathBuf,
    pub pages: Vec<Page>,
    pub(crate) route_page_indices: BTreeMap<String, usize>,
    pub indexed_paths: BTreeSet<String>,
    pub inbound_links: BTreeMap<String, BTreeSet<String>>,
    pub llms_text: Option<String>,
    pub robots_text: Option<String>,
    pub sitemap_routes: BTreeSet<String>,
    pub sitemap_error: Option<String>,
    pub deployment_model: DeploymentModel,
    pub deployment_markers: Vec<String>,
    pub crawl_meta: Option<CrawlMeta>,
}

impl Page {
    pub fn meta_description(&self) -> Option<&str> {
        self.meta_by_name.get("description").map(String::as_str)
    }

    pub fn metadata(&self, key: &str) -> Option<&str> {
        let lower = key.to_ascii_lowercase();
        self.meta_by_name
            .get(&lower)
            .or_else(|| self.meta_by_property.get(&lower))
            .map(String::as_str)
    }
}

impl Site {
    pub fn page(&self, route: &str) -> Option<&Page> {
        self.route_page_indices
            .get(route)
            .and_then(|index| self.pages.get(*index))
    }

    pub fn has_route(&self, route: &str) -> bool {
        self.route_page_indices.contains_key(route)
    }

    pub fn route_pages(&self) -> impl Iterator<Item = &Page> {
        self.route_page_indices
            .values()
            .filter_map(|index| self.pages.get(*index))
    }

    pub fn route_keys(&self) -> impl Iterator<Item = &String> {
        self.route_page_indices.keys()
    }

    pub fn route_page_pairs(&self) -> impl Iterator<Item = (&String, &Page)> {
        self.route_page_indices
            .iter()
            .filter_map(|(route, index)| self.pages.get(*index).map(|page| (route, page)))
    }
}

fn iter_html_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            iter_html_files(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("html") {
            out.push(path);
        }
    }
    Ok(())
}

pub(crate) fn capture_route_for_relative_path(relative: &str) -> String {
    if relative == "index.html" {
        return String::new();
    }
    if let Some(prefix) = relative.strip_suffix("/index.html") {
        return prefix.to_string();
    }
    if let Some(prefix) = relative.strip_suffix(".html") {
        return prefix.to_string();
    }
    relative.to_string()
}

fn is_search_route(route: &str) -> bool {
    route == "search"
        || route.starts_with("search/")
        || route.ends_with("/search")
        || route.contains("/search/")
}

fn classify_page_kind(relative: &str, route: &str) -> PageKind {
    if relative == "404.html" || route == "404" {
        return PageKind::NotFound;
    }
    if route.is_empty() {
        return PageKind::Home;
    }
    let lower = route.to_ascii_lowercase();
    if lower.starts_with("admin") || lower.contains("/admin/") {
        return PageKind::Admin;
    }
    if [
        "legal",
        "privacy",
        "terms",
        "mentions-legales",
        "politique-de-confidentialite",
    ]
    .iter()
    .any(|candidate| lower == *candidate || lower.starts_with(&format!("{}/", candidate)))
    {
        return PageKind::Legal;
    }
    if lower.ends_with(".xml") || lower.starts_with("feed") || lower.ends_with("/feed") {
        return PageKind::Feed;
    }
    if lower.starts_with("docs/")
        || lower.starts_with("guide")
        || lower.contains("/docs/")
        || lower.contains("/guide/")
    {
        return PageKind::Docs;
    }
    if lower == "skills"
        || lower == "features"
        || lower == "maintainers"
        || lower.starts_with("category/")
        || lower.starts_with("tag/")
        || lower.starts_with("maintainer/")
    {
        return PageKind::Listing;
    }
    if lower.starts_with("skill/")
        || lower.starts_with("features/")
        || lower.starts_with("product/")
        || lower.starts_with("service/")
    {
        return PageKind::Detail;
    }
    if is_search_route(&lower) {
        return PageKind::Search;
    }
    if ["submit", "ops", "status"].contains(&lower.as_str()) {
        return PageKind::Utility;
    }
    PageKind::Generic
}

pub fn normalize_internal_href(href: &str) -> Option<String> {
    if !href.starts_with('/') || href.starts_with("//") {
        return None;
    }
    let cleaned = href
        .split('#')
        .next()
        .unwrap_or(href)
        .split('?')
        .next()
        .unwrap_or(href);
    if cleaned == "/" {
        return Some(String::new());
    }
    let trimmed = cleaned.trim_start_matches('/').trim_end_matches('/');
    Some(trimmed.to_string())
}

pub fn route_from_urlish(value: &str) -> Option<String> {
    if let Some(route) = normalize_internal_href(value) {
        return Some(route);
    }
    let after_scheme = value
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(value);
    let path = after_scheme
        .find('/')
        .map(|index| &after_scheme[index..])
        .unwrap_or("/");
    normalize_internal_href(path)
}

pub(crate) fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn strip_tags(raw: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    collapse_whitespace(&result)
}

fn line_column_for(raw: &str, index: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for ch in raw[..index.min(raw.len())].chars() {
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn build_site_index(root: &Path) -> Result<BTreeSet<String>> {
    let mut indexed = BTreeSet::new();
    collect_site_index(root, root, &mut indexed)?;
    Ok(indexed)
}

fn collect_site_index(root: &Path, dir: &Path, indexed: &mut BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_site_index(root, &path, indexed)?;
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        indexed.insert(relative.clone());
        if relative == "index.html" {
            indexed.insert(String::new());
        }
        if let Some(prefix) = relative.strip_suffix("/index.html") {
            indexed.insert(prefix.to_string());
            indexed.insert(format!("{}/", prefix));
        } else if let Some(prefix) = relative.strip_suffix(".html") {
            indexed.insert(prefix.to_string());
        }
    }
    Ok(())
}

fn prefer_page(current: Option<&Page>, candidate: &Page) -> bool {
    let Some(current) = current else {
        return true;
    };
    let current_is_clean =
        current.relative_path == "index.html" || current.relative_path.ends_with("/index.html");
    let candidate_is_clean =
        candidate.relative_path == "index.html" || candidate.relative_path.ends_with("/index.html");
    if candidate_is_clean && !current_is_clean {
        return true;
    }
    candidate_is_clean == current_is_clean
        && candidate.relative_path.len() < current.relative_path.len()
}

fn build_inbound_link_map(pages: &[Page]) -> BTreeMap<String, BTreeSet<String>> {
    let mut inbound = BTreeMap::new();
    for page in pages {
        inbound
            .entry(page.route.clone())
            .or_insert_with(BTreeSet::new);
    }
    for page in pages {
        for target in &page.internal_links {
            inbound
                .entry(target.clone())
                .or_insert_with(BTreeSet::new)
                .insert(page.relative_path.clone());
        }
    }
    inbound
}

fn read_optional_text(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn detect_deployment_model(root: &Path) -> (DeploymentModel, Vec<String>) {
    if root == Path::new("crawl") {
        return (
            DeploymentModel::RuntimeSnapshot,
            vec!["runtime snapshot root".to_string()],
        );
    }

    let mut markers = Vec::new();
    for (candidate, label) in [
        ("_worker.js", "cloudflare worker bundle"),
        ("server", "server runtime output"),
        (".vercel/output", "vercel output"),
        ("functions", "functions runtime directory"),
    ] {
        if root.join(candidate).exists() {
            markers.push(label.to_string());
        }
    }

    if markers.is_empty() {
        (DeploymentModel::StaticExport, markers)
    } else {
        (DeploymentModel::SsrWorker, markers)
    }
}

fn build_indexed_paths(
    pages: &[Page],
    artifacts: impl IntoIterator<Item = &'static str>,
) -> BTreeSet<String> {
    let mut indexed = BTreeSet::new();
    for page in pages {
        indexed.insert(page.relative_path.clone());
        if page.relative_path == "index.html" {
            indexed.insert(String::new());
        }
        if let Some(prefix) = page.relative_path.strip_suffix("/index.html") {
            indexed.insert(prefix.to_string());
            indexed.insert(format!("{}/", prefix));
        } else if let Some(prefix) = page.relative_path.strip_suffix(".html") {
            indexed.insert(prefix.to_string());
        }
    }
    for artifact in artifacts {
        indexed.insert(artifact.to_string());
    }
    indexed
}

pub fn build_page_from_source(
    path: PathBuf,
    relative_path: String,
    raw: String,
    response_headers: BTreeMap<String, String>,
) -> Page {
    parser::build_page_from_source(path, relative_path, raw, response_headers)
}

pub struct SiteArtifacts {
    pub llms_text: Option<String>,
    pub robots_text: Option<String>,
    pub sitemap_text: Option<String>,
}

pub struct SiteBuildInput {
    pub root: PathBuf,
    pub pages: Vec<Page>,
    pub artifacts: SiteArtifacts,
    pub deployment_model: DeploymentModel,
    pub deployment_markers: Vec<String>,
    pub crawl_meta: Option<CrawlMeta>,
}

pub fn build_site_from_parts(input: SiteBuildInput) -> Result<Site> {
    let SiteBuildInput {
        root,
        pages,
        artifacts,
        deployment_model,
        deployment_markers,
        crawl_meta,
    } = input;
    let mut route_page_indices = BTreeMap::new();
    for (index, page) in pages.iter().enumerate() {
        let current = route_page_indices
            .get(&page.route)
            .and_then(|existing| pages.get(*existing));
        if prefer_page(current, page) {
            route_page_indices.insert(page.route.clone(), index);
        }
    }
    let (sitemap_routes, sitemap_error) = match artifacts.sitemap_text {
        Some(text) => match sitemap::read_sitemap_routes_from_text(&text) {
            Ok(routes) => (routes, None),
            Err(error) => (BTreeSet::new(), Some(error.to_string())),
        },
        None => (BTreeSet::new(), None),
    };
    let indexed_paths = build_indexed_paths(&pages, ["robots.txt", "llms.txt", "sitemap.xml"]);
    Ok(Site {
        root,
        inbound_links: build_inbound_link_map(&pages),
        llms_text: artifacts.llms_text,
        robots_text: artifacts.robots_text,
        sitemap_routes,
        sitemap_error,
        deployment_model,
        deployment_markers,
        crawl_meta,
        pages,
        route_page_indices,
        indexed_paths,
    })
}

pub fn load_site(root: &Path) -> Result<Site> {
    let mut html_files = Vec::new();
    iter_html_files(root, &mut html_files)?;
    html_files.sort();
    let pages: Vec<Page> = html_files
        .iter()
        .map(|path| parser::parse_page_from_file(path, root))
        .collect::<Result<Vec<_>>>()?;
    let (deployment_model, deployment_markers) = detect_deployment_model(root);
    let mut site = build_site_from_parts(SiteBuildInput {
        root: root.to_path_buf(),
        pages,
        artifacts: SiteArtifacts {
            llms_text: read_optional_text(&root.join("llms.txt")),
            robots_text: read_optional_text(&root.join("robots.txt")),
            sitemap_text: read_optional_text(&root.join("sitemap.xml")),
        },
        deployment_model,
        deployment_markers,
        crawl_meta: None,
    })?;
    site.indexed_paths = build_site_index(root)?;
    let (sitemap_routes, sitemap_error) =
        sitemap::read_sitemap_routes(&root.join("sitemap.xml"), root);
    site.sitemap_routes = sitemap_routes;
    site.sitemap_error = sitemap_error;
    Ok(site)
}

#[cfg(test)]
mod tests {
    use super::{DeploymentModel, PageKind, load_site, normalize_internal_href};
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn normalizes_internal_hrefs() {
        assert_eq!(normalize_internal_href("/"), Some(String::new()));
        assert_eq!(
            normalize_internal_href("/guide/"),
            Some("guide".to_string())
        );
        assert_eq!(
            normalize_internal_href("/guide/page.html?x=1#hash"),
            Some("guide/page.html".to_string())
        );
        assert_eq!(normalize_internal_href("https://example.com"), None);
    }

    #[test]
    fn loads_clean_route_pages_and_nested_sitemap() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html lang=\"en\"><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"><meta property=\"og:title\" content=\"Open Graph\"></head><body><h1>x</h1><section data-ui=\"hero\"><h2>Hero</h2><p>Body text</p></section><script type=\"application/ld+json\">{\"@type\":\"WebPage\",\"name\":\"x\"}</script><a href=\"/guide\">Learn more</a></body></html>",
        );
        write(
            &root.join("guide/index.html"),
            "<html lang=\"en\"><head><title>Guide</title><meta name=\"description\" content=\"Guide desc\"><link rel=\"canonical\" href=\"https://example.com/guide\"><link rel=\"alternate\" hreflang=\"fr\" href=\"/fr/guide\"></head><body><h1>Guide</h1><details><summary>Q</summary><p>A</p></details><pre><code>ls</code></pre></body></html>",
        );
        write(
            &root.join("sitemap.xml"),
            "<sitemapindex><sitemap><loc>https://example.com/sitemap-core.xml</loc></sitemap></sitemapindex>",
        );
        write(
            &root.join("sitemap-core.xml"),
            "<urlset><url><loc>https://example.com/guide</loc></url></urlset>",
        );
        write(&root.join("llms.txt"), "# Site\n\n## Pages\n");
        let site = load_site(root).unwrap();
        assert!(site.has_route(""));
        assert!(site.has_route("guide"));
        assert!(site.indexed_paths.contains("guide"));
        assert!(site.sitemap_routes.contains("guide"));
        assert!(site.llms_text.is_some());
        let home = site.page("").unwrap();
        assert_eq!(
            home.meta_by_property.get("og:title").map(String::as_str),
            Some("Open Graph")
        );
        assert_eq!(home.links.len(), 1);
        assert_eq!(home.h1_texts, vec!["x".to_string()]);
        assert_eq!(home.blocks.len(), 1);
        assert_eq!(home.json_ld_blocks.len(), 1);
        let inbound = site.inbound_links.get("guide").unwrap();
        assert!(inbound.contains("index.html"));
        let guide = site.page("guide").unwrap();
        assert_eq!(guide.alternate_links.len(), 1);
        assert_eq!(guide.details_blocks.len(), 1);
        assert_eq!(guide.pre_blocks.len(), 1);
    }

    #[test]
    fn keeps_feature_routes_with_search_terms_as_detail_pages() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("features/terminal-search/index.html"),
            "<html><head><title>Terminal Search | Chau7 Terminal</title><meta name=\"description\" content=\"Search feature\"><link rel=\"canonical\" href=\"https://example.com/features/terminal-search\"></head><body><h1>Terminal Search</h1></body></html>",
        );
        let site = load_site(root).unwrap();
        let page = site.page("features/terminal-search").unwrap();
        assert_eq!(page.page_kind, PageKind::Detail);
    }

    #[test]
    fn classifies_search_routes_by_route_shape_not_substrings() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("search/index.html"),
            "<html><head><title>Search</title><meta name=\"description\" content=\"Find things\"></head><body><h1>Search</h1></body></html>",
        );
        let site = load_site(root).unwrap();
        let page = site.page("search").unwrap();
        assert_eq!(page.page_kind, PageKind::Search);
    }

    #[test]
    fn detects_worker_style_deployment_outputs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("404.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"></head><body><h1>x</h1></body></html>",
        );
        write(&root.join("_worker.js/index.js"), "export default {};");
        let site = load_site(root).unwrap();
        assert_eq!(site.deployment_model, DeploymentModel::SsrWorker);
        assert!(!site.deployment_markers.is_empty());
    }
}
