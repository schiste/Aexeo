use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::site::{Page, Site, normalize_internal_href, route_from_urlish};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineSurfaceKind {
    Html,
    JsonLd,
    Facts,
    LlmsTxt,
    LlmsFullTxt,
    MarkdownMirror,
    Sitemap,
    Robots,
    Feed,
    OpenGraph,
    TwitterCard,
    IndexNowKey,
    ExternalAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineSurfaceDiscoverySource {
    PageMetadata,
    StructuredData,
    StaticLink,
    RenderedLink,
    LlmsIndex,
    Sitemap,
    Robots,
    ConventionProbe,
    LocalArtifact,
    Config,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineSurfaceStatus {
    Present,
    Missing,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSurface {
    pub kind: MachineSurfaceKind,
    pub status: MachineSurfaceStatus,
    pub discovery_source: MachineSurfaceDiscoverySource,
    pub url: Option<String>,
    pub path: Option<String>,
    pub route: Option<String>,
    pub source_route: Option<String>,
    pub confidence: u8,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MachineSurfaceCoverage {
    pub total_routes: usize,
    pub routes_with_schema: usize,
    pub routes_with_markdown_mirror: usize,
    pub routes_with_static_machine_link: usize,
    pub routes_in_sitemap: usize,
    pub llms_present: bool,
    pub llms_full_present: bool,
    pub facts_present: bool,
    pub robots_present: bool,
    pub sitemap_present: bool,
    pub static_machine_links: usize,
    pub llms_index_links: usize,
    pub convention_probe_hits: usize,
    pub convention_probe_misses: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSurfaceRouteSummary {
    pub route: String,
    pub canonical: Option<String>,
    pub schema_types: Vec<String>,
    pub markdown_mirrors: Vec<String>,
    pub static_machine_links: Vec<String>,
    pub sitemap_listed: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSurfaceGraph {
    pub site_root: String,
    pub site_url: Option<String>,
    pub surfaces: Vec<MachineSurface>,
    pub coverage: MachineSurfaceCoverage,
    pub routes: Vec<MachineSurfaceRouteSummary>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct MachineSurfaceOptions<'a> {
    pub site_url: Option<&'a str>,
}

impl<'a> MachineSurfaceOptions<'a> {
    pub fn new(site_url: Option<&'a str>) -> Self {
        Self { site_url }
    }
}

fn normalize_site_url(site_url: Option<&str>) -> Option<String> {
    site_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('/').to_string())
}

fn absolute_url(site_url: Option<&str>, href: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    let site_url = normalize_site_url(site_url)?;
    if href == "/" {
        return Some(format!("{site_url}/"));
    }
    Some(format!("{}/{}", site_url, href.trim_start_matches('/')))
}

fn route_url(site_url: Option<&str>, route: &str) -> Option<String> {
    let site_url = normalize_site_url(site_url)?;
    if route.is_empty() {
        Some(format!("{site_url}/"))
    } else {
        Some(format!("{site_url}/{route}"))
    }
}

fn route_display_path(route: &str) -> String {
    if route.is_empty() {
        "/".to_string()
    } else {
        format!("/{route}")
    }
}

fn markdown_links(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut offset = 0;
    while let Some(start_rel) = text[offset..].find("](") {
        let start = offset + start_rel + 2;
        let Some(end_rel) = text[start..].find(')') else {
            break;
        };
        let end = start + end_rel;
        let candidate = text[start..end]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim();
        if !candidate.is_empty() {
            links.push(candidate.to_string());
        }
        offset = end + 1;
    }
    links
}

fn attr_machine_kind(href: &str) -> Option<MachineSurfaceKind> {
    let lowered = href
        .split('#')
        .next()
        .unwrap_or(href)
        .split('?')
        .next()
        .unwrap_or(href)
        .to_ascii_lowercase();
    if lowered.ends_with("/llms.txt") || lowered == "llms.txt" || lowered.ends_with("llms.txt") {
        return Some(MachineSurfaceKind::LlmsTxt);
    }
    if lowered.ends_with("/llms-full.txt")
        || lowered == "llms-full.txt"
        || lowered.ends_with("llms-full.txt")
    {
        return Some(MachineSurfaceKind::LlmsFullTxt);
    }
    if lowered.ends_with("/facts.json")
        || lowered == "facts.json"
        || lowered.ends_with("/aexeo-truth.json")
        || lowered.ends_with("aexeo-truth.json")
    {
        return Some(MachineSurfaceKind::Facts);
    }
    if lowered.ends_with("/sitemap.xml") || lowered == "sitemap.xml" {
        return Some(MachineSurfaceKind::Sitemap);
    }
    if lowered.ends_with("/robots.txt") || lowered == "robots.txt" {
        return Some(MachineSurfaceKind::Robots);
    }
    if lowered.ends_with(".md") || lowered.ends_with(".md.txt") {
        return Some(MachineSurfaceKind::MarkdownMirror);
    }
    if lowered.ends_with(".rss")
        || lowered.ends_with(".atom")
        || lowered.ends_with("/feed")
        || lowered.ends_with("/feed.xml")
        || lowered.ends_with("rss.xml")
    {
        return Some(MachineSurfaceKind::Feed);
    }
    None
}

fn collect_schema_types(page: &Page) -> Vec<String> {
    let mut types = BTreeSet::new();
    for block in &page.json_ld_blocks {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&block.raw) else {
            continue;
        };
        collect_schema_types_from_value(&value, &mut types);
    }
    types.into_iter().collect()
}

fn collect_schema_types_from_value(value: &serde_json::Value, types: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(value) = map.get("@type") {
                match value {
                    serde_json::Value::String(item) => {
                        types.insert(item.clone());
                    }
                    serde_json::Value::Array(items) => {
                        for item in items {
                            if let serde_json::Value::String(text) = item {
                                types.insert(text.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
            for nested in map.values() {
                collect_schema_types_from_value(nested, types);
            }
        }
        serde_json::Value::Array(items) => {
            for nested in items {
                collect_schema_types_from_value(nested, types);
            }
        }
        _ => {}
    }
}

fn markdown_mirror_candidates(route: &str) -> Vec<String> {
    if route.is_empty() {
        return vec![
            "index.md".to_string(),
            "index.md.txt".to_string(),
            "index.html.md".to_string(),
            "index.html.md.txt".to_string(),
        ];
    }
    vec![
        format!("{route}.md"),
        format!("{route}.md.txt"),
        format!("{route}/index.md"),
        format!("{route}/index.md.txt"),
        format!("{route}/index.html.md"),
        format!("{route}/index.html.md.txt"),
    ]
}

fn local_artifact_exists(site: &Site, path: &str) -> bool {
    site.indexed_paths.contains(path) || site.root.join(path).exists()
}

fn add_surface(surfaces: &mut Vec<MachineSurface>, surface: MachineSurface) {
    if surfaces.iter().any(|existing| {
        existing.kind == surface.kind
            && existing.status == surface.status
            && existing.discovery_source == surface.discovery_source
            && existing.url == surface.url
            && existing.path == surface.path
            && existing.route == surface.route
            && existing.source_route == surface.source_route
    }) {
        return;
    }
    surfaces.push(surface);
}

fn add_local_artifact(
    site: &Site,
    surfaces: &mut Vec<MachineSurface>,
    kind: MachineSurfaceKind,
    path: &str,
    site_url: Option<&str>,
    note: &str,
) -> bool {
    if !local_artifact_exists(site, path) {
        return false;
    }
    add_surface(
        surfaces,
        MachineSurface {
            kind,
            status: MachineSurfaceStatus::Present,
            discovery_source: MachineSurfaceDiscoverySource::LocalArtifact,
            url: absolute_url(site_url, &format!("/{path}")),
            path: Some(path.to_string()),
            route: None,
            source_route: None,
            confidence: 100,
            notes: vec![note.to_string()],
        },
    );
    true
}

fn linked_route_for_href(href: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        route_from_urlish(href)
    } else {
        normalize_internal_href(&format!("/{}", href.trim_start_matches('/')))
    }
}

fn discover_page_surfaces(
    site: &Site,
    page: &Page,
    site_url: Option<&str>,
    surfaces: &mut Vec<MachineSurface>,
) -> MachineSurfaceRouteSummary {
    let route = page.route.clone();
    let schema_types = collect_schema_types(page);
    let mut markdown_mirrors = Vec::new();
    let mut static_machine_links = Vec::new();

    add_surface(
        surfaces,
        MachineSurface {
            kind: MachineSurfaceKind::Html,
            status: MachineSurfaceStatus::Present,
            discovery_source: MachineSurfaceDiscoverySource::PageMetadata,
            url: page
                .canonical
                .clone()
                .or_else(|| route_url(site_url, &route)),
            path: Some(page.relative_path.clone()),
            route: Some(route.clone()),
            source_route: None,
            confidence: 100,
            notes: vec!["canonical HTML page loaded into the site graph".to_string()],
        },
    );

    if !schema_types.is_empty() {
        add_surface(
            surfaces,
            MachineSurface {
                kind: MachineSurfaceKind::JsonLd,
                status: MachineSurfaceStatus::Present,
                discovery_source: MachineSurfaceDiscoverySource::StructuredData,
                url: page
                    .canonical
                    .clone()
                    .or_else(|| route_url(site_url, &route)),
                path: Some(page.relative_path.clone()),
                route: Some(route.clone()),
                source_route: None,
                confidence: 95,
                notes: vec![format!("schema types: {}", schema_types.join(", "))],
            },
        );
    }

    if page
        .meta_by_property
        .keys()
        .any(|key| key.starts_with("og:"))
    {
        add_surface(
            surfaces,
            MachineSurface {
                kind: MachineSurfaceKind::OpenGraph,
                status: MachineSurfaceStatus::Present,
                discovery_source: MachineSurfaceDiscoverySource::PageMetadata,
                url: page
                    .canonical
                    .clone()
                    .or_else(|| route_url(site_url, &route)),
                path: Some(page.relative_path.clone()),
                route: Some(route.clone()),
                source_route: None,
                confidence: 85,
                notes: vec!["OpenGraph metadata present".to_string()],
            },
        );
    }

    if page.meta_by_name.contains_key("twitter:card") {
        add_surface(
            surfaces,
            MachineSurface {
                kind: MachineSurfaceKind::TwitterCard,
                status: MachineSurfaceStatus::Present,
                discovery_source: MachineSurfaceDiscoverySource::PageMetadata,
                url: page
                    .canonical
                    .clone()
                    .or_else(|| route_url(site_url, &route)),
                path: Some(page.relative_path.clone()),
                route: Some(route.clone()),
                source_route: None,
                confidence: 85,
                notes: vec!["Twitter/X card metadata present".to_string()],
            },
        );
    }

    for link in &page.links {
        let Some(kind) = attr_machine_kind(&link.href) else {
            continue;
        };
        let url = absolute_url(site_url, &link.href).or_else(|| Some(link.href.clone()));
        if matches!(kind, MachineSurfaceKind::MarkdownMirror)
            && let Some(url) = &url
        {
            markdown_mirrors.push(url.clone());
        }
        if let Some(url) = &url {
            static_machine_links.push(url.clone());
        }
        add_surface(
            surfaces,
            MachineSurface {
                kind,
                status: MachineSurfaceStatus::Unverified,
                discovery_source: MachineSurfaceDiscoverySource::StaticLink,
                url,
                path: linked_route_for_href(&link.href),
                route: None,
                source_route: Some(route.clone()),
                confidence: 80,
                notes: vec![format!(
                    "linked from {} with anchor text '{}'",
                    route_display_path(&route),
                    link.text
                )],
            },
        );
    }

    // <link rel="alternate" type="text/markdown" href="..."> discovery
    // tags also count as static machine links — that's the whole point
    // of the manifest-driven discovery-link injection in `aexeo-cli fix`.
    // Without this loop, SRF005 fires on pages where the fix has already
    // wired up the discovery link, which contradicts the fix's intent.
    for alternate in &page.alternate_links {
        let Some(type_attr) = alternate.type_attr.as_deref() else {
            continue;
        };
        let kind = match type_attr.to_ascii_lowercase().as_str() {
            "text/markdown" => MachineSurfaceKind::MarkdownMirror,
            _ => continue,
        };
        let url = absolute_url(site_url, &alternate.href).or_else(|| Some(alternate.href.clone()));
        if matches!(kind, MachineSurfaceKind::MarkdownMirror)
            && let Some(url) = &url
        {
            markdown_mirrors.push(url.clone());
        }
        if let Some(url) = &url {
            static_machine_links.push(url.clone());
        }
        add_surface(
            surfaces,
            MachineSurface {
                kind,
                status: MachineSurfaceStatus::Unverified,
                discovery_source: MachineSurfaceDiscoverySource::StaticLink,
                url,
                path: linked_route_for_href(&alternate.href),
                route: None,
                source_route: Some(route.clone()),
                confidence: 85,
                notes: vec![format!(
                    "linked from {} via <link rel=\"alternate\" type=\"text/markdown\">",
                    route_display_path(&route)
                )],
            },
        );
    }

    for candidate in markdown_mirror_candidates(&route) {
        if local_artifact_exists(site, &candidate) {
            let href = format!("/{candidate}");
            let url = absolute_url(site_url, &href).unwrap_or(href);
            markdown_mirrors.push(url.clone());
            add_surface(
                surfaces,
                MachineSurface {
                    kind: MachineSurfaceKind::MarkdownMirror,
                    status: MachineSurfaceStatus::Present,
                    discovery_source: MachineSurfaceDiscoverySource::ConventionProbe,
                    url: Some(url),
                    path: Some(candidate),
                    route: Some(route.clone()),
                    source_route: None,
                    confidence: 70,
                    notes: vec![
                        "Markdown mirror exists at a conventional URL but may not be statically linked"
                            .to_string(),
                    ],
                },
            );
        }
    }

    markdown_mirrors.sort();
    markdown_mirrors.dedup();
    static_machine_links.sort();
    static_machine_links.dedup();

    let mut issues = Vec::new();
    if schema_types.is_empty() {
        issues.push("no JSON-LD schema detected".to_string());
    }
    if markdown_mirrors.is_empty() {
        issues.push("no Markdown mirror discovered by link or convention".to_string());
    }
    if static_machine_links.is_empty() {
        issues.push("no static machine-readable links discovered on the page".to_string());
    }
    if !site.sitemap_routes.is_empty() && !site.sitemap_routes.contains(&route) {
        issues.push("route is absent from sitemap.xml".to_string());
    }

    MachineSurfaceRouteSummary {
        route,
        canonical: page.canonical.clone(),
        schema_types,
        markdown_mirrors,
        static_machine_links,
        sitemap_listed: site.sitemap_routes.contains(&page.route),
        issues,
    }
}

fn add_llms_index_surfaces(
    site: &Site,
    site_url: Option<&str>,
    surfaces: &mut Vec<MachineSurface>,
) {
    let Some(text) = site.llms_text.as_deref() else {
        return;
    };
    for href in markdown_links(text) {
        let Some(kind) = attr_machine_kind(&href) else {
            continue;
        };
        let path = linked_route_for_href(&href);
        let local_status = path
            .as_deref()
            .map(|path| {
                if local_artifact_exists(site, path) {
                    MachineSurfaceStatus::Present
                } else {
                    MachineSurfaceStatus::Missing
                }
            })
            .unwrap_or(MachineSurfaceStatus::Unverified);
        add_surface(
            surfaces,
            MachineSurface {
                kind,
                status: local_status,
                discovery_source: MachineSurfaceDiscoverySource::LlmsIndex,
                url: absolute_url(site_url, &href).or_else(|| Some(href.clone())),
                path,
                route: None,
                source_route: None,
                confidence: 90,
                notes: vec!["referenced from llms.txt".to_string()],
            },
        );
    }
}

fn add_sitemap_surfaces(site: &Site, site_url: Option<&str>, surfaces: &mut Vec<MachineSurface>) {
    for route in &site.sitemap_routes {
        add_surface(
            surfaces,
            MachineSurface {
                kind: MachineSurfaceKind::Html,
                status: if site.has_route(route) {
                    MachineSurfaceStatus::Present
                } else {
                    MachineSurfaceStatus::Unverified
                },
                discovery_source: MachineSurfaceDiscoverySource::Sitemap,
                url: route_url(site_url, route),
                path: Some(route.clone()),
                route: Some(route.clone()),
                source_route: None,
                confidence: 75,
                notes: vec!["listed in sitemap.xml".to_string()],
            },
        );
    }
}

fn recommendations_for(
    coverage: &MachineSurfaceCoverage,
    routes: &[MachineSurfaceRouteSummary],
) -> Vec<String> {
    let mut recommendations = Vec::new();
    if !coverage.facts_present {
        recommendations.push(
            "publish /facts.json as the portable structured baseline for organization, product, offer, and canonical claim facts"
                .to_string(),
        );
    }
    if !coverage.llms_present {
        recommendations.push(
            "publish /llms.txt to provide an explicit curated index for LLM and agent retrieval"
                .to_string(),
        );
    }
    if coverage.total_routes > 0 && coverage.routes_with_markdown_mirror < coverage.total_routes {
        recommendations.push(format!(
            "add Markdown mirrors for high-value pages first; current coverage is {}/{} routes",
            coverage.routes_with_markdown_mirror, coverage.total_routes
        ));
    }
    if coverage.routes_with_static_machine_link < coverage.routes_with_markdown_mirror {
        recommendations.push(
            "link Markdown mirrors from HTML or /llms.txt so discovery does not depend only on convention probes"
                .to_string(),
        );
    }
    if coverage.total_routes > 0 && coverage.routes_with_schema < coverage.total_routes {
        recommendations.push(format!(
            "expand JSON-LD coverage; {} route(s) currently lack structured data",
            coverage.total_routes - coverage.routes_with_schema
        ));
    }
    if !coverage.sitemap_present {
        recommendations.push(
            "publish sitemap.xml so classic search crawlers can discover canonical HTML inventory"
                .to_string(),
        );
    }
    if routes
        .iter()
        .any(|route| !route.markdown_mirrors.is_empty() && route.static_machine_links.is_empty())
    {
        recommendations.push(
            "surface generated Markdown mirrors with static anchors or llms.txt entries instead of relying only on generated UI"
                .to_string(),
        );
    }
    recommendations
}

fn build_coverage(
    site: &Site,
    surfaces: &[MachineSurface],
    routes: &[MachineSurfaceRouteSummary],
) -> MachineSurfaceCoverage {
    let route_count = routes.len();
    MachineSurfaceCoverage {
        total_routes: route_count,
        routes_with_schema: routes
            .iter()
            .filter(|route| !route.schema_types.is_empty())
            .count(),
        routes_with_markdown_mirror: routes
            .iter()
            .filter(|route| !route.markdown_mirrors.is_empty())
            .count(),
        routes_with_static_machine_link: routes
            .iter()
            .filter(|route| !route.static_machine_links.is_empty())
            .count(),
        routes_in_sitemap: routes.iter().filter(|route| route.sitemap_listed).count(),
        llms_present: site.llms_text.is_some(),
        llms_full_present: local_artifact_exists(site, "llms-full.txt"),
        facts_present: ["facts.json", ".well-known/facts.json", "aexeo-truth.json"]
            .iter()
            .any(|path| local_artifact_exists(site, path)),
        robots_present: site.robots_text.is_some(),
        sitemap_present: site.sitemap_text.is_some(),
        static_machine_links: surfaces
            .iter()
            .filter(|surface| surface.discovery_source == MachineSurfaceDiscoverySource::StaticLink)
            .count(),
        llms_index_links: surfaces
            .iter()
            .filter(|surface| surface.discovery_source == MachineSurfaceDiscoverySource::LlmsIndex)
            .count(),
        convention_probe_hits: surfaces
            .iter()
            .filter(|surface| {
                surface.discovery_source == MachineSurfaceDiscoverySource::ConventionProbe
                    && surface.status == MachineSurfaceStatus::Present
            })
            .count(),
        convention_probe_misses: route_count.saturating_sub(
            routes
                .iter()
                .filter(|route| !route.markdown_mirrors.is_empty())
                .count(),
        ),
    }
}

pub fn discover_machine_surface_graph(
    site: &Site,
    options: MachineSurfaceOptions<'_>,
) -> MachineSurfaceGraph {
    let site_url = normalize_site_url(options.site_url);
    let mut surfaces = Vec::new();

    if site.llms_text.is_some() {
        add_surface(
            &mut surfaces,
            MachineSurface {
                kind: MachineSurfaceKind::LlmsTxt,
                status: MachineSurfaceStatus::Present,
                discovery_source: MachineSurfaceDiscoverySource::LocalArtifact,
                url: absolute_url(site_url.as_deref(), "/llms.txt"),
                path: Some("llms.txt".to_string()),
                route: None,
                source_route: None,
                confidence: 100,
                notes: vec!["root llms.txt loaded".to_string()],
            },
        );
    }
    if site.robots_text.is_some() {
        add_surface(
            &mut surfaces,
            MachineSurface {
                kind: MachineSurfaceKind::Robots,
                status: MachineSurfaceStatus::Present,
                discovery_source: MachineSurfaceDiscoverySource::LocalArtifact,
                url: absolute_url(site_url.as_deref(), "/robots.txt"),
                path: Some("robots.txt".to_string()),
                route: None,
                source_route: None,
                confidence: 100,
                notes: vec!["root robots.txt loaded".to_string()],
            },
        );
    }
    if site.sitemap_text.is_some() {
        add_surface(
            &mut surfaces,
            MachineSurface {
                kind: MachineSurfaceKind::Sitemap,
                status: MachineSurfaceStatus::Present,
                discovery_source: MachineSurfaceDiscoverySource::LocalArtifact,
                url: absolute_url(site_url.as_deref(), "/sitemap.xml"),
                path: Some("sitemap.xml".to_string()),
                route: None,
                source_route: None,
                confidence: 100,
                notes: vec!["root sitemap.xml loaded".to_string()],
            },
        );
    }

    for (kind, path, note) in [
        (
            MachineSurfaceKind::LlmsFullTxt,
            "llms-full.txt",
            "root llms-full.txt loaded",
        ),
        (
            MachineSurfaceKind::Facts,
            "facts.json",
            "root facts manifest loaded",
        ),
        (
            MachineSurfaceKind::Facts,
            ".well-known/facts.json",
            "well-known facts manifest loaded",
        ),
        (
            MachineSurfaceKind::Facts,
            "aexeo-truth.json",
            "legacy Aexeo truth manifest loaded",
        ),
    ] {
        add_local_artifact(site, &mut surfaces, kind, path, site_url.as_deref(), note);
    }

    let mut routes = site
        .route_page_pairs()
        .map(|(_, page)| discover_page_surfaces(site, page, site_url.as_deref(), &mut surfaces))
        .collect::<Vec<_>>();
    routes.sort_by(|left, right| left.route.cmp(&right.route));

    add_llms_index_surfaces(site, site_url.as_deref(), &mut surfaces);
    add_sitemap_surfaces(site, site_url.as_deref(), &mut surfaces);

    surfaces.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.route.cmp(&right.route))
            .then_with(|| left.url.cmp(&right.url))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.discovery_source.cmp(&right.discovery_source))
    });

    let coverage = build_coverage(site, &surfaces, &routes);
    let recommendations = recommendations_for(&coverage, &routes);

    MachineSurfaceGraph {
        site_root: site.root.to_string_lossy().into_owned(),
        site_url,
        surfaces,
        coverage,
        routes,
        recommendations,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MachineSurfaceDiscoverySource, MachineSurfaceKind, MachineSurfaceOptions,
        MachineSurfaceStatus, discover_machine_surface_graph,
    };
    use crate::site::load_site;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn discovers_local_machine_surfaces_and_convention_mirrors() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            r#"<html><head>
                <title>Home</title>
                <meta name="description" content="Home">
                <meta property="og:title" content="Home">
                <meta name="twitter:card" content="summary">
                <link rel="canonical" href="https://example.com/">
                <script type="application/ld+json">{"@type":"Organization","name":"Example"}</script>
            </head><body><h1>Home</h1><a href="/index.md.txt">View as Markdown</a></body></html>"#,
        );
        write(
            &root.join("about/index.html"),
            r#"<html><head><title>About</title><meta name="description" content="About"></head><body><h1>About</h1></body></html>"#,
        );
        write(&root.join("index.md.txt"), "# Home\n");
        write(&root.join("facts.json"), "{}\n");
        write(
            &root.join("llms.txt"),
            "# Example\n\n## Pages\n- [Home](/index.md.txt)\n",
        );
        write(
            &root.join("sitemap.xml"),
            "<urlset><url><loc>https://example.com/</loc></url></urlset>",
        );
        write(&root.join("robots.txt"), "User-agent: *\nAllow: /\n");

        let site = load_site(root).unwrap();
        let graph = discover_machine_surface_graph(
            &site,
            MachineSurfaceOptions::new(Some("https://example.com")),
        );

        assert!(graph.coverage.facts_present);
        assert!(graph.coverage.llms_present);
        assert_eq!(graph.coverage.routes_with_markdown_mirror, 1);
        assert!(graph.surfaces.iter().any(|surface| {
            surface.kind == MachineSurfaceKind::MarkdownMirror
                && surface.discovery_source == MachineSurfaceDiscoverySource::ConventionProbe
                && surface.status == MachineSurfaceStatus::Present
        }));
        assert!(
            graph
                .routes
                .iter()
                .find(|route| route.route == "about")
                .unwrap()
                .issues
                .iter()
                .any(|issue| issue.contains("Markdown mirror"))
        );
    }

    #[test]
    fn treats_llms_references_as_discovery_edges() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("llms.txt"),
            "# Example\n\n## Docs\n- [Missing](/missing.md.txt)\n",
        );
        let site = load_site(root).unwrap();
        let graph = discover_machine_surface_graph(&site, MachineSurfaceOptions::new(None));
        let surface = graph
            .surfaces
            .iter()
            .find(|surface| surface.path.as_deref() == Some("missing.md.txt"))
            .unwrap();
        assert_eq!(
            surface.discovery_source,
            MachineSurfaceDiscoverySource::LlmsIndex
        );
        assert_eq!(surface.status, MachineSurfaceStatus::Missing);
    }
}
