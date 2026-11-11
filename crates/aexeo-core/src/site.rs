#[path = "site/markup.rs"]
pub(crate) mod markup;
#[path = "site/parser.rs"]
mod parser;
pub(crate) use parser::style_hides_content;
#[path = "site/sitemap.rs"]
mod sitemap;

use crate::error::UserContext;
use aexeo_contracts::{AuditCrawlMeta, AuditPageSnapshot, AuditSiteSnapshot};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use url::Url;

pub(crate) use parser::audited_link_targets;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlternateLink {
    pub href: String,
    pub hreflang: Option<String>,
    /// `type` attribute when present (e.g. `"text/markdown"` for the
    /// markdown-mirror discovery link Aexeo's fix injects). Optional
    /// because the i18n-style `hreflang` alternates this struct was
    /// originally for don't carry one.
    pub type_attr: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CrawlMeta {
    pub visited_pages: usize,
    pub max_pages: usize,
    pub discovered_internal_routes: usize,
    pub truncated: bool,
    /// Well-known machine-readable artifact paths that were
    /// HEAD-probed during the crawl and returned 2xx. Folded into
    /// `Site::indexed_paths` at site-build time so surface
    /// discovery sees them as present even when they're not
    /// reachable from the HTML link graph.
    pub probed_artifact_paths: Vec<String>,
    /// Requests answered with a non-2xx status. These are not pages.
    pub failed_responses: Vec<aexeo_contracts::CrawlResponseProvenance>,
    /// URLs the crawler did not request because robots.txt disallows them.
    pub robots_skipped_urls: Vec<String>,
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
    /// Additional `srcset` candidates associated with this image.
    pub asset_sources: Vec<String>,
    /// AVIF/WebP candidates supplied by a modern `<source>` inside `<picture>`.
    pub modern_sources: Vec<String>,
    pub alt: Option<String>,
    pub line: usize,
    pub column: usize,
    /// `role` attribute on the `<img>` if any. Captured for the
    /// A11Y "smart" alt-text mode: an image with role="presentation"
    /// or role="none" is the canonical author signal that the image
    /// is decorative and should be skipped by missing-alt checks.
    pub role: Option<String>,
    /// True iff the `<img>` had `aria-hidden="true"`. Same purpose
    /// as `role`: marks the image as not-conveying-information so
    /// strict missing-alt checks can be relaxed.
    pub aria_hidden: bool,
    /// Intrinsic HTML attributes used to reserve layout space.
    pub width: Option<String>,
    pub height: Option<String>,
    /// True when the enclosing `<picture>` has an AVIF or WebP source.
    pub has_modern_source: bool,
    /// Whether the image belongs to a `<figure>` and its visible caption, if any.
    pub inside_figure: bool,
    pub figure_caption: Option<String>,
}

impl ImageReference {
    /// True when the author has explicitly marked this image as
    /// decorative — either via `alt=""` (the canonical HTML form),
    /// `role="presentation"` / `role="none"`, or `aria-hidden="true"`.
    /// The A11Y001 rule consults this in default ("smart") mode to
    /// avoid flagging properly-tagged decorative imagery; `--strict`
    /// mode bypasses this method and only treats `alt=""` as decorative.
    pub fn is_marked_decorative(&self) -> bool {
        if matches!(self.alt.as_deref(), Some("")) {
            return true;
        }
        if self.aria_hidden {
            return true;
        }
        matches!(
            self.role.as_deref().map(str::trim),
            Some("presentation") | Some("none")
        )
    }
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
    pub crawl_response: Option<aexeo_contracts::CrawlResponseProvenance>,
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
    pub sitemap_text: Option<String>,
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

/// Test-only view of this module's private classifier, so the
/// characterization test can compare it against the generator's.
#[cfg(test)]
pub(crate) fn classify_page_kind_for_test(route: &str) -> String {
    format!("{:?}", classify_page_kind(route, route))
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

pub(crate) fn decode_url_path(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset] == b'%' {
            let high = hex_digit(*bytes.get(offset + 1)?)?;
            let low = hex_digit(*bytes.get(offset + 2)?)?;
            let byte = (high << 4) | low;
            if byte == 0 {
                return None;
            }
            decoded.push(byte);
            offset += 3;
        } else {
            decoded.push(bytes[offset]);
            offset += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn sitemap_image_urls(
    root: &Path,
    allowed_origin: Option<&str>,
) -> anyhow::Result<BTreeSet<String>> {
    sitemap::read_sitemap_image_urls(&root.join("sitemap.xml"), root, allowed_origin)
}

pub(crate) fn sitemap_image_urls_from_text(text: &str) -> Option<BTreeSet<String>> {
    sitemap::read_sitemap_image_urls_from_text(text)
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
    let mut rest = raw;
    while let Some(open) = rest.find('<') {
        result.push_str(&rest[..open]);
        let tag = &rest[open + 1..];
        let name = tag.trim_start_matches('/');
        let name_end = name
            .find(|ch: char| !ch.is_ascii_alphanumeric())
            .unwrap_or(name.len());
        if markup::tag_breaks_text(&name[..name_end]) {
            result.push(' ');
        }
        // An unclosed `<` drops the rest, as it did when scanning by character.
        let Some(close) = tag.find('>') else {
            rest = "";
            break;
        };
        rest = &tag[close + 1..];
    }
    result.push_str(rest);
    collapse_whitespace(&result)
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
    fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
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

/// Give a crawled page the link targets its observed URL makes resolvable.
///
/// The source parser only knows root-relative hrefs, because a static file has
/// no URL. A crawled page does, so the links the crawl followed must also count
/// for inbound and broken-link analysis, or a page reached through a relative
/// link would look orphaned. Links the parser already located keep their line.
fn apply_crawled_links(page: &mut Page) {
    let Some(url) = page
        .crawl_response
        .as_ref()
        .and_then(|response| response.observed_url.as_deref())
    else {
        return;
    };
    for crate::link_evidence::CrawlLink {
        href,
        anchor,
        route,
    } in crate::link_evidence::crawl_links(&page.raw_text, url)
    {
        let mut located = false;
        for link in page.links.iter_mut().filter(|link| link.href == href) {
            located = true;
            link.target.get_or_insert_with(|| route.clone());
        }
        if !located {
            page.links.push(Link {
                href,
                target: Some(route),
                text: anchor,
                line: 1,
                column: 1,
            });
        }
    }
    page.internal_links = page
        .links
        .iter()
        .filter_map(|link| link.target.clone())
        .collect();
}

pub fn build_site_from_parts(input: SiteBuildInput) -> Result<Site> {
    let SiteBuildInput {
        root,
        mut pages,
        artifacts,
        deployment_model,
        deployment_markers,
        crawl_meta,
    } = input;
    pages.iter_mut().for_each(apply_crawled_links);
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
        Some(ref text) => match sitemap::read_sitemap_routes_from_text(text) {
            Ok(routes) => (routes, None),
            Err(error) => (BTreeSet::new(), Some(error.to_string())),
        },
        None => (BTreeSet::new(), None),
    };
    let mut indexed_paths = build_indexed_paths(&pages, ["robots.txt", "llms.txt", "sitemap.xml"]);
    // Probed artifact paths from the live crawl (facts.json,
    // llms-full.txt, manifest-listed siblings) get folded into
    // indexed_paths so surface discovery and rules consulting
    // `site.indexed_paths.contains(...)` find them. Without this,
    // artifacts not reachable via the HTML link graph appear
    // "missing" in surface coverage even when curl returns 200.
    if let Some(meta) = crawl_meta.as_ref() {
        for path in &meta.probed_artifact_paths {
            indexed_paths.insert(path.clone());
        }
        // A route the crawler was told not to request or denied access to is
        // unknown, not broken.
        let denied = meta
            .failed_responses
            .iter()
            .filter(|response| matches!(response.status_code, Some(401 | 403)))
            .map(|response| &response.requested_url);
        indexed_paths.extend(
            meta.robots_skipped_urls
                .iter()
                .chain(denied)
                .filter_map(|url| route_from_urlish(url)),
        );
    }
    Ok(Site {
        root,
        inbound_links: build_inbound_link_map(&pages),
        llms_text: artifacts.llms_text,
        robots_text: artifacts.robots_text,
        sitemap_text: artifacts.sitemap_text.clone(),
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

fn deployment_model_label(model: &DeploymentModel) -> &'static str {
    match model {
        DeploymentModel::StaticExport => "static_export",
        DeploymentModel::SsrWorker => "ssr_worker",
        DeploymentModel::RuntimeSnapshot => "runtime_snapshot",
    }
}

fn deployment_model_from_label(label: &str) -> DeploymentModel {
    match label {
        "ssr_worker" => DeploymentModel::SsrWorker,
        "runtime_snapshot" => DeploymentModel::RuntimeSnapshot,
        _ => DeploymentModel::StaticExport,
    }
}

pub fn audit_site_snapshot(site: &Site, site_url: Option<&str>) -> AuditSiteSnapshot {
    AuditSiteSnapshot {
        version: 1,
        site_url: site_url.map(crate::link_graph::redact_url),
        root: site.root.to_string_lossy().to_string(),
        deployment_model: deployment_model_label(&site.deployment_model).to_string(),
        deployment_markers: site.deployment_markers.clone(),
        crawl_meta: site.crawl_meta.as_ref().map(|meta| AuditCrawlMeta {
            visited_pages: meta.visited_pages,
            max_pages: meta.max_pages,
            discovered_internal_routes: meta.discovered_internal_routes,
            truncated: meta.truncated,
            probed_artifact_paths: meta.probed_artifact_paths.clone(),
            failed_responses: meta.failed_responses.clone(),
            robots_skipped_urls: meta.robots_skipped_urls.clone(),
        }),
        llms_text: site.llms_text.clone(),
        robots_text: site.robots_text.clone(),
        sitemap_text: site.sitemap_text.clone(),
        pages: site
            .pages
            .iter()
            .map(|page| AuditPageSnapshot {
                path: page.path.to_string_lossy().to_string(),
                relative_path: page.relative_path.clone(),
                route: page.route.clone(),
                raw_html: page.raw_text.clone(),
                response_headers: page.response_headers.clone(),
                crawl_response: page.crawl_response.clone(),
            })
            .collect(),
    }
}

pub fn build_site_from_audit_snapshot(snapshot: &AuditSiteSnapshot) -> Result<Site> {
    let pages = snapshot
        .pages
        .iter()
        .map(|page| {
            let mut built = build_page_from_source(
                PathBuf::from(&page.path),
                page.relative_path.clone(),
                page.raw_html.clone(),
                page.response_headers.clone(),
            );
            built.crawl_response = page.crawl_response.clone();
            built
        })
        .collect::<Vec<_>>();
    build_site_from_parts(SiteBuildInput {
        root: PathBuf::from(&snapshot.root),
        pages,
        artifacts: SiteArtifacts {
            llms_text: snapshot.llms_text.clone(),
            robots_text: snapshot.robots_text.clone(),
            sitemap_text: snapshot.sitemap_text.clone(),
        },
        deployment_model: deployment_model_from_label(&snapshot.deployment_model),
        deployment_markers: snapshot.deployment_markers.clone(),
        crawl_meta: snapshot.crawl_meta.as_ref().map(|meta| CrawlMeta {
            visited_pages: meta.visited_pages,
            max_pages: meta.max_pages,
            discovered_internal_routes: meta.discovered_internal_routes,
            truncated: meta.truncated,
            probed_artifact_paths: meta.probed_artifact_paths.clone(),
            failed_responses: meta.failed_responses.clone(),
            robots_skipped_urls: meta.robots_skipped_urls.clone(),
        }),
    })
}

fn page_sitemap_origin(page: &Page) -> Option<String> {
    let value = page
        .canonical
        .as_deref()
        .or_else(|| page.meta_by_property.get("og:url").map(String::as_str))?;
    let url = Url::parse(value.trim()).ok()?;
    matches!(url.scheme(), "http" | "https")
        .then(|| url.origin().ascii_serialization().to_ascii_lowercase())
}

fn primary_site_sitemap_origin(
    pages: &[Page],
    configured_site_url: Option<&str>,
) -> Option<String> {
    pages
        .iter()
        .find(|page| page.route.is_empty())
        .and_then(page_sitemap_origin)
        .or_else(|| configured_site_url.and_then(site_url_sitemap_origin))
        .or_else(|| pages.iter().find_map(page_sitemap_origin))
}

fn site_url_sitemap_origin(site_url: &str) -> Option<String> {
    let url = Url::parse(site_url.trim()).ok()?;
    matches!(url.scheme(), "http" | "https")
        .then(|| url.origin().ascii_serialization().to_ascii_lowercase())
}

pub fn load_site(root: &Path) -> Result<Site> {
    load_site_with_site_url(root, None)
}

/// Load a static site using the configured site URL when interpreting its sitemap.
pub fn load_site_with_site_url(root: &Path, site_url: Option<&str>) -> Result<Site> {
    read_site(root, site_url).user_context(format!("cannot load site at {}", root.display()))
}

fn read_site(root: &Path, site_url: Option<&str>) -> Result<Site> {
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
    let allowed_origin = primary_site_sitemap_origin(&site.pages, site_url);
    let (sitemap_routes, sitemap_error) =
        sitemap::read_sitemap_routes(&root.join("sitemap.xml"), root, allowed_origin.as_deref());
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
    fn captures_image_dimensions_picture_formats_and_visible_figure_captions() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><body><picture><source type=\"image/avif\" srcset=\"/images/chair.avif\"><img src=\"/images/chair.jpg\" alt=\"Office chair\" width=\"640\" height=\"480\"></picture><figure><img src=\"/images/desk.jpg\" alt=\"Desk setup\"><figcaption>A desk with a chair and monitor.</figcaption></figure><IMG SRC = \"/images/alternate.png\" ALT = \"Alternate asset\" WIDTH = \"320\" HEIGHT = \"240\"><img src=\"/images/angle>asset.png\" alt=\"Angle bracket asset\" width=\"320\" height=\"240\"><!-- <img src=\"/images/commented.webp\" alt=\"Commented\"> --><script>const html = '<img src=\"/images/script.webp\" alt=\"Script\">';</script><img-widget src=\"/images/widget.webp\" alt=\"Widget\"></body></html>",
        );

        let site = load_site(root).unwrap();
        let images = &site.page("").unwrap().images;

        assert_eq!(images.len(), 4);
        assert_eq!(images[0].width.as_deref(), Some("640"));
        assert_eq!(images[0].height.as_deref(), Some("480"));
        assert!(images[0].has_modern_source);
        assert!(!images[0].inside_figure);
        assert!(images[1].inside_figure);
        assert_eq!(
            images[1].figure_caption.as_deref(),
            Some("A desk with a chair and monitor.")
        );
        assert_eq!(images[2].src, "/images/alternate.png");
        assert_eq!(images[2].alt.as_deref(), Some("Alternate asset"));
        assert_eq!(images[2].width.as_deref(), Some("320"));
        assert_eq!(images[2].height.as_deref(), Some("240"));
        assert_eq!(images[3].src, "/images/angle>asset.png");
        assert_eq!(images[3].alt.as_deref(), Some("Angle bracket asset"));
    }

    #[test]
    fn captures_image_when_srcset_is_the_only_img_source() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><body><img srcset=\"/images/responsive-1x.jpg 1x, /images/responsive-2x.jpg 2x\" alt=\"Responsive photo\" width=\"320\" height=\"240\"></body></html>",
        );

        let site = load_site(root).unwrap();
        let images = &site.page("").unwrap().images;

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].src, "/images/responsive-1x.jpg");
    }

    #[test]
    fn image_context_tags_ignore_case_and_attributes_match_whole_names() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><body><PICTURE><Source data-srcset=\"/images/lazy.webp\" SRCSET=\"/images/chair.avif 1x\"><img data-src=\"/x.jpg\" src=\"/images/chair.jpg\" alt=\"Chair\"></PICTURE>\
             <FIGURE><figure><img src=\"/images/inner.jpg\" alt=\"Inner\"></figure><img src=\"/images/outer.jpg\" alt=\"Outer\"><FigCaption title=\"x>y\">Outer <b>caption</b></FigCaption></FIGURE>\
             <img src=\"/images/after.jpg\" alt=\"After\"></body></html>",
        );

        let site = load_site(root).unwrap();
        let images = &site.page("").unwrap().images;

        assert_eq!(images.len(), 4);
        assert_eq!(images[0].src, "/images/chair.jpg");
        assert_eq!(images[0].modern_sources, ["/images/chair.avif"]);
        assert!(
            images[0]
                .asset_sources
                .iter()
                .all(|source| source != "/images/lazy.webp")
        );
        assert!(images[1].inside_figure && images[1].figure_caption.is_none());
        assert!(images[2].inside_figure);
        assert_eq!(images[2].figure_caption.as_deref(), Some("Outer caption"));
        assert!(!images[3].inside_figure && !images[3].has_modern_source);
    }

    #[test]
    fn crafted_figure_markup_neither_panics_nor_invents_a_caption() {
        // (markup, whether the image is inside a figure)
        for (html, inside_figure) in [
            (
                "<figure title=\"</figure>\"><img src=\"/a.jpg\" alt=\"A\"></figure>",
                true,
            ),
            (
                "<figure title=\"</figure><figcaption>Fake</figcaption>\"><img src=\"/a.jpg\" alt=\"A\">",
                true,
            ),
            (
                "<img src=\"/a.jpg\" alt=\"<figure>\"></figure><figcaption>Fake</figcaption>",
                false,
            ),
            ("</figure><img src=\"/a.jpg\" alt=\"A\"><figure", false),
            (
                "<figure><img src=\"/a.jpg\" alt=\"A\"><figcaption title=\"</figcaption>\">",
                true,
            ),
        ] {
            let page = super::parser::build_page_from_source(
                std::path::PathBuf::from("index.html"),
                "index.html".to_string(),
                format!("<html><body>{html}</body></html>"),
                std::collections::BTreeMap::new(),
            );
            assert_eq!(page.images.len(), 1, "{html}");
            assert_eq!(page.images[0].inside_figure, inside_figure, "{html}");
            assert_eq!(page.images[0].figure_caption, None, "{html}");
        }
    }

    #[test]
    fn ignores_script_markup_and_figure_tag_prefixes_when_associating_images() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><body><script>const fake = '<picture><source type=\"image/webp\"><figure>';</script><img src=\"/images/legacy-photo.jpg\" alt=\"Legacy photo\"><figure-widget><img src=\"/images/widget-photo.jpg\" alt=\"Widget photo\"></figure-widget></body></html>",
        );

        let site = load_site(root).unwrap();
        let images = &site.page("").unwrap().images;

        assert_eq!(images.len(), 2);
        assert!(!images[0].has_modern_source);
        assert!(!images[1].inside_figure);
    }

    #[test]
    fn does_not_read_nested_route_sitemaps_from_another_origin() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>Home</h1></body></html>",
        );
        write(
            &root.join("sitemap.xml"),
            "<sitemapindex><sitemap><loc>https://evil.example/sitemaps/images</loc></sitemap></sitemapindex>",
        );
        write(
            &root.join("sitemaps/images"),
            "<urlset><url><loc>https://example.com/leaked-route</loc></url></urlset>",
        );

        let site = load_site(root).unwrap();

        assert!(!site.sitemap_routes.contains("leaked-route"));
        assert!(site.sitemap_error.is_some());
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
            "<s:sitemapindex xmlns:s=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><s:sitemap><s:loc>https://example.com/sitemap-core.xml</s:loc></s:sitemap></s:sitemapindex>",
        );
        write(
            &root.join("sitemap-core.xml"),
            "<s:urlset xmlns:s=\"http://www.sitemaps.org/schemas/sitemap/0.9\" xmlns:img=\"http://www.google.com/schemas/sitemap-image/1.1\"><s:url><s:loc>https://example.com/guide</s:loc><img:image><img:loc>https://example.com/images/picture.webp</img:loc></img:image></s:url></s:urlset>",
        );
        write(&root.join("llms.txt"), "# Site\n\n## Pages\n");
        let site = load_site(root).unwrap();
        assert!(site.has_route(""));
        assert!(site.has_route("guide"));
        assert!(site.indexed_paths.contains("guide"));
        assert!(site.sitemap_routes.contains("guide"));
        assert!(!site.sitemap_routes.contains("images/picture.webp"));
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

    fn only_page(html: &[u8]) -> super::Page {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(temp_dir.path().join("index.html"), html).unwrap();
        let site = load_site(temp_dir.path()).unwrap();
        site.page("").unwrap().clone()
    }

    #[test]
    fn title_and_h1_are_found_whatever_the_tag_case() {
        let page = only_page(
            b"<HTML><HEAD><TITLE>Upper</TITLE></HEAD><BODY><H1 CLASS=\"a\">Head</H1></BODY></HTML>",
        );
        assert_eq!(page.title.as_deref(), Some("Upper"));
        assert_eq!(page.h1_texts, vec!["Head".to_string()]);
    }

    #[test]
    fn comments_scripts_templates_and_svg_titles_are_not_markup() {
        let page = only_page(
            b"<html><head><!-- <title>Old</title> --><title>Live</title><script>x='<h1>s</h1>'</script></head>\
              <body><svg><title>Icon</title></svg><!-- <h1>Old</h1> --><h1>Live</h1><template><h1>t</h1></template></body></html>",
        );
        assert_eq!(page.title.as_deref(), Some("Live"));
        assert_eq!(page.h1_texts, vec!["Live".to_string()]);
        assert_eq!(page.h1_count, 1);
    }

    #[test]
    fn svg_title_is_not_the_page_title() {
        let page = only_page(b"<html><body><svg><title>Icon</title></svg><h1>x</h1></body></html>");
        assert_eq!(page.title, None);
    }

    #[test]
    fn title_and_h1_text_decode_entities_and_keep_nested_text() {
        let page = only_page(b"<title>A &amp; B</title><h1>One <em>two</em>\n three</h1>");
        assert_eq!(page.title.as_deref(), Some("A & B"));
        assert_eq!(page.h1_texts, vec!["One two three".to_string()]);
    }

    #[test]
    fn empty_h1_does_not_count() {
        let page = only_page(b"<title>x</title><h1> </h1>");
        assert_eq!(page.h1_count, 0);
    }

    #[test]
    fn line_numbers_of_other_elements_are_unaffected_by_title_extraction() {
        let page = only_page(b"<!-- c -->\n<TITLE>x</TITLE>\n<H1>y</H1>\n<a href=\"/z\">z</a>");
        assert_eq!(page.links[0].line, 4);
    }

    #[test]
    fn non_utf8_page_is_decoded_lossily_and_the_site_keeps_loading() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::write(root.join("index.html"), "<title>Home</title><h1>Home</h1>").unwrap();
        fs::write(
            root.join("latin.html"),
            b"<title>Caf\xe9</title><h1>Caf\xe9</h1>",
        )
        .unwrap();

        let site = load_site(root).unwrap();

        assert!(site.has_route(""));
        assert_eq!(
            site.page("latin").unwrap().title.as_deref(),
            Some("Caf\u{fffd}")
        );
    }

    #[test]
    fn non_utf8_robots_still_counts_as_present() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::write(root.join("index.html"), "<title>Home</title>").unwrap();
        fs::write(
            root.join("robots.txt"),
            b"User-agent: *\nDisallow: /caf\xe9\n",
        )
        .unwrap();

        let site = load_site(root).unwrap();

        assert!(site.robots_text.is_some());
    }

    /// React and Next.js write `hrefLang`, and the attribute must still be read.
    #[test]
    fn a_page_reads_camel_case_attributes_and_ignores_script_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.html"),
            concat!(
                r#"<html LANG="en"><head><title>x</title>"#,
                r#"<link rel="alternate" hrefLang="x-default" href="/"/>"#,
                r#"<link data-rel="canonical" rel="canonical" href="/real"/>"#,
                r#"<meta property="og:title" content="a > b"/>"#,
                r#"<script>var a = '<a href="/in-script">x</a><img src="/in-script.png">';</script>"#,
                r#"<!-- <a href="/in-comment">x</a> --></head><body><h1>x</h1>"#,
                r#"<article><p>text</p></article>"#,
                r#"<section>before<script>var t = "code";</script><style>.a{}</style></section>"#,
                r#"<a data-href="/wrong" href="/right">y</a></body></html>"#,
            ),
        )
        .unwrap();
        let site = load_site(dir.path()).unwrap();
        let page = site.page("").unwrap();

        assert_eq!(
            page.alternate_links[0].hreflang.as_deref(),
            Some("x-default")
        );
        assert_eq!(page.html_lang.as_deref(), Some("en"));
        assert_eq!(page.canonical.as_deref(), Some("/real"));
        assert_eq!(page.meta_by_property["og:title"], "a > b");
        let hrefs: Vec<_> = page.links.iter().map(|link| link.href.as_str()).collect();
        assert_eq!(hrefs, ["/right"]);
        assert!(page.images.is_empty());
        let section = page
            .blocks
            .iter()
            .find(|block| block.tag == "section")
            .unwrap();
        assert_eq!(section.text, "before");
    }

    #[test]
    fn block_and_break_tags_separate_words_but_inline_tags_do_not() {
        assert_eq!(
            super::strip_tags("document<br>eligibility"),
            "document eligibility"
        );
        assert_eq!(super::strip_tags("<div>a</div><div>b</div>"), "a b");
        assert_eq!(super::strip_tags("<li>a</li><LI>b</LI>"), "a b");
        assert_eq!(super::strip_tags("wo<b>r</b>d <span>x</span>"), "word x");
        assert_eq!(super::strip_tags("a < b"), "a");

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.html"),
            "<html><head><title>x</title></head><body><h1>Secure your<br/>wo<b>r</b>ld<div>today</div></h1></body></html>",
        )
        .unwrap();
        let site = load_site(dir.path()).unwrap();
        assert_eq!(site.page("").unwrap().h1_texts, ["Secure your world today"]);
    }
}
