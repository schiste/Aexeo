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
    pub route_pages: BTreeMap<String, Page>,
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

fn html_route_for(relative: &str) -> String {
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

fn parse_page(path: &Path, root: &Path) -> Result<Page> {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let raw = fs::read_to_string(path)?;
    let (meta_by_name, meta_by_property) = capture_meta_maps(&raw);
    let links = capture_links(&raw);
    let internal_links = links
        .iter()
        .filter_map(|link| link.target.clone())
        .collect();
    let h1_texts = capture_paired_tag_texts(&raw, "h1");
    let route = html_route_for(&relative);
    Ok(Page {
        path: path.to_path_buf(),
        relative_path: relative.clone(),
        route: route.clone(),
        page_kind: classify_page_kind(&relative, &route),
        raw_text: raw.clone(),
        title: capture_tag_text(&raw, "title"),
        meta_by_name,
        meta_by_property,
        canonical: capture_canonical_href(&raw),
        html_lang: capture_html_lang(&raw),
        h1_count: h1_texts.len(),
        h1_texts,
        has_breadcrumb_nav: capture_breadcrumb_nav(&raw),
        response_headers: BTreeMap::new(),
        links,
        internal_links,
        alternate_links: capture_alternate_links(&raw),
        images: capture_images(&raw),
        blocks: capture_semantic_blocks(&raw),
        details_blocks: capture_details_blocks(&raw),
        pre_blocks: capture_pre_blocks(&raw),
        json_ld_blocks: capture_json_ld_blocks(&raw),
    })
}

fn capture_tag_text(raw: &str, tag: &str) -> Option<String> {
    capture_paired_tag_texts(raw, tag).into_iter().next()
}

fn capture_paired_tag_texts(raw: &str, tag: &str) -> Vec<String> {
    let mut texts = Vec::new();
    let start_marker = format!("<{}", tag);
    let end_marker = format!("</{}>", tag);
    let mut offset = 0;
    while let Some(index) = raw[offset..].find(&start_marker) {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let Some(close_start_rel) = raw[open_end + 1..].find(&end_marker) else {
            break;
        };
        let close_start = open_end + 1 + close_start_rel;
        let text = strip_tags(&raw[open_end + 1..close_start]);
        if !text.is_empty() {
            texts.push(text);
        }
        offset = close_start + end_marker.len();
    }
    texts
}

fn capture_meta_maps(raw: &str) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let mut by_name = BTreeMap::new();
    let mut by_property = BTreeMap::new();
    for snippet in tag_snippets(raw, "meta") {
        let content = attr_value(&snippet, "content").unwrap_or_default();
        if content.is_empty() {
            continue;
        }
        if let Some(name) = attr_value(&snippet, "name") {
            by_name.insert(name.to_ascii_lowercase(), content.clone());
        }
        if let Some(property) = attr_value(&snippet, "property") {
            by_property.insert(property.to_ascii_lowercase(), content.clone());
        }
    }
    (by_name, by_property)
}

fn capture_canonical_href(raw: &str) -> Option<String> {
    for snippet in tag_snippets(raw, "link") {
        let rel = attr_value(&snippet, "rel")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if rel.split_whitespace().any(|part| part == "canonical") {
            return attr_value(&snippet, "href");
        }
    }
    None
}

fn capture_alternate_links(raw: &str) -> Vec<AlternateLink> {
    let mut links = Vec::new();
    for snippet in tag_snippets(raw, "link") {
        let rel = attr_value(&snippet, "rel")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !rel.split_whitespace().any(|part| part == "alternate") {
            continue;
        }
        let Some(href) = attr_value(&snippet, "href") else {
            continue;
        };
        let hreflang = attr_value(&snippet, "hreflang").filter(|value| !value.is_empty());
        links.push(AlternateLink { href, hreflang });
    }
    links
}

fn capture_links(raw: &str) -> Vec<Link> {
    let mut links = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<a") {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let snippet = &raw[start..=open_end];
        let Some(href) = attr_value(snippet, "href") else {
            offset = open_end + 1;
            continue;
        };
        let Some(close_start_rel) = raw[open_end + 1..].find("</a>") else {
            offset = open_end + 1;
            continue;
        };
        let close_start = open_end + 1 + close_start_rel;
        let text = strip_tags(&raw[open_end + 1..close_start]);
        let (line, column) = line_column_for(raw, start);
        links.push(Link {
            href: href.clone(),
            target: normalize_internal_href(&href),
            text,
            line,
            column,
        });
        offset = close_start + 4;
    }
    links
}

fn capture_html_lang(raw: &str) -> Option<String> {
    let start = raw.find("<html")?;
    let end = raw[start..].find('>')? + start;
    attr_value(&raw[start..=end], "lang").filter(|value| !value.is_empty())
}

fn capture_breadcrumb_nav(raw: &str) -> bool {
    raw.to_ascii_lowercase().contains("breadcrumb")
}

fn capture_images(raw: &str) -> Vec<ImageReference> {
    let mut images = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<img") {
        let start = offset + index;
        let Some(end_rel) = raw[start..].find('>') else {
            break;
        };
        let end = start + end_rel;
        let snippet = &raw[start..=end];
        let Some(src) = attr_value(snippet, "src") else {
            offset = end + 1;
            continue;
        };
        let alt = attr_value(snippet, "alt");
        let (line, column) = line_column_for(raw, start);
        images.push(ImageReference {
            src,
            alt,
            line,
            column,
        });
        offset = end + 1;
    }
    images
}

fn capture_semantic_blocks(raw: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    blocks.extend(capture_named_blocks(raw, "section"));
    blocks.extend(capture_named_blocks(raw, "article"));
    blocks.sort_by_key(|block| (block.line, block.column));
    blocks
}

fn capture_named_blocks(raw: &str, tag: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let start_marker = format!("<{}", tag);
    let end_marker = format!("</{}>", tag);
    let mut offset = 0;
    while let Some(index) = raw[offset..].find(&start_marker) {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let snippet = &raw[start..=open_end];
        let Some(close_start_rel) = raw[open_end + 1..].find(&end_marker) else {
            offset = open_end + 1;
            continue;
        };
        let close_start = open_end + 1 + close_start_rel;
        let inner = &raw[open_end + 1..close_start];
        let (line, column) = line_column_for(raw, start);
        blocks.push(Block {
            tag: tag.to_string(),
            data_ui: attr_value(snippet, "data-ui"),
            line,
            column,
            has_heading: has_heading_tag(inner),
            text: strip_tags(inner),
        });
        offset = close_start + end_marker.len();
    }
    blocks
}

fn has_heading_tag(raw: &str) -> bool {
    (1..=6).any(|level| raw.contains(&format!("<h{}", level)))
}

fn capture_details_blocks(raw: &str) -> Vec<DetailsBlock> {
    let mut blocks = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<details") {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let Some(close_start_rel) = raw[open_end + 1..].find("</details>") else {
            offset = open_end + 1;
            continue;
        };
        let close_start = open_end + 1 + close_start_rel;
        let inner = &raw[open_end + 1..close_start];
        let (line, column) = line_column_for(raw, start);
        blocks.push(DetailsBlock {
            line,
            column,
            has_summary: inner.contains("<summary"),
        });
        offset = close_start + 10;
    }
    blocks
}

fn capture_pre_blocks(raw: &str) -> Vec<PreBlock> {
    let mut blocks = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<pre") {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let Some(close_start_rel) = raw[open_end + 1..].find("</pre>") else {
            offset = open_end + 1;
            continue;
        };
        let close_start = open_end + 1 + close_start_rel;
        let inner = &raw[open_end + 1..close_start];
        let (line, column) = line_column_for(raw, start);
        blocks.push(PreBlock {
            line,
            column,
            has_code: inner.contains("<code"),
        });
        offset = close_start + 6;
    }
    blocks
}

fn capture_json_ld_blocks(raw: &str) -> Vec<JsonLdBlock> {
    let mut blocks = Vec::new();
    let mut offset = 0;
    while let Some(index) = raw[offset..].find("<script") {
        let start = offset + index;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        let snippet = &raw[start..=open_end];
        let script_type = attr_value(snippet, "type")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let Some(close_start_rel) = raw[open_end + 1..].find("</script>") else {
            offset = open_end + 1;
            continue;
        };
        let close_start = open_end + 1 + close_start_rel;
        if script_type == "application/ld+json" {
            let (line, column) = line_column_for(raw, start);
            blocks.push(JsonLdBlock {
                raw: raw[open_end + 1..close_start].trim().to_string(),
                line,
                column,
            });
        }
        offset = close_start + 9;
    }
    blocks
}

fn tag_snippets(raw: &str, tag: &str) -> Vec<String> {
    let mut snippets = Vec::new();
    let needle = format!("<{}", tag);
    let mut offset = 0;
    while let Some(index) = raw[offset..].find(&needle) {
        let start = offset + index;
        let Some(end_rel) = raw[start..].find('>') else {
            break;
        };
        let end = start + end_rel;
        snippets.push(raw[start..=end].to_string());
        offset = end + 1;
    }
    snippets
}

fn attr_value(snippet: &str, attr: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!("{}={}", attr, quote);
        if let Some(start) = snippet.find(&needle) {
            let value_start = start + needle.len();
            let end = snippet[value_start..].find(quote)? + value_start;
            return Some(snippet[value_start..end].trim().to_string());
        }
    }
    None
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
    if !text.contains("<urlset") {
        anyhow::bail!("invalid sitemap XML")
    }
    Ok(read_loc_values(&text)
        .into_iter()
        .filter_map(|value| route_from_urlish(&value))
        .collect())
}

fn read_sitemap_routes(path: &Path, root: &Path) -> (BTreeSet<String>, Option<String>) {
    if !path.exists() {
        return (BTreeSet::new(), None);
    }
    let mut seen = BTreeSet::new();
    match read_sitemap_routes_recursive(path, root, &mut seen) {
        Ok(routes) => (routes, None),
        Err(error) => (BTreeSet::new(), Some(error.to_string())),
    }
}

pub fn load_site(root: &Path) -> Result<Site> {
    let mut html_files = Vec::new();
    iter_html_files(root, &mut html_files)?;
    html_files.sort();
    let pages: Vec<Page> = html_files
        .iter()
        .map(|path| parse_page(path, root))
        .collect::<Result<Vec<_>>>()?;

    let mut route_pages = BTreeMap::new();
    for page in &pages {
        if prefer_page(route_pages.get(&page.route), page) {
            route_pages.insert(page.route.clone(), page.clone());
        }
    }

    let (sitemap_routes, sitemap_error) = read_sitemap_routes(&root.join("sitemap.xml"), root);
    let (deployment_model, deployment_markers) = detect_deployment_model(root);
    Ok(Site {
        root: root.to_path_buf(),
        inbound_links: build_inbound_link_map(&pages),
        llms_text: read_optional_text(&root.join("llms.txt")),
        robots_text: read_optional_text(&root.join("robots.txt")),
        sitemap_routes,
        sitemap_error,
        deployment_model,
        deployment_markers,
        crawl_meta: None,
        pages,
        route_pages,
        indexed_paths: build_site_index(root)?,
    })
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
        assert!(site.route_pages.contains_key(""));
        assert!(site.route_pages.contains_key("guide"));
        assert!(site.indexed_paths.contains("guide"));
        assert!(site.sitemap_routes.contains("guide"));
        assert!(site.llms_text.is_some());
        let home = site.route_pages.get("").unwrap();
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
        let guide = site.route_pages.get("guide").unwrap();
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
        let page = site.route_pages.get("features/terminal-search").unwrap();
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
        let page = site.route_pages.get("search").unwrap();
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
