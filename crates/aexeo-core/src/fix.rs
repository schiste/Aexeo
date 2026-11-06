use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::config::Config;
use crate::generate::{
    LLMS_GENERATED_MARKER, build_link_suggestions, render_bundle_llms_txt, render_llms_txt,
    render_robots_txt,
};
use crate::site::{Page, Site, load_site};

/// A file `fix` left untouched, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFix {
    pub path: PathBuf,
    pub reason: String,
}

/// Outcome of one `fix` run. Under `FixOptions::dry_run`, `changed` lists the
/// files a real run would write and nothing has been written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixReport {
    pub changed: Vec<PathBuf>,
    pub skipped: Vec<SkippedFix>,
}

/// In-memory result of every fix pass. Passes never touch the disk, so a dry
/// run reports exactly what a real run writes, and a pass that fails leaves
/// the site as it was.
#[derive(Default)]
struct FixPlan {
    pending: BTreeMap<PathBuf, String>,
    skipped: BTreeMap<PathBuf, String>,
}

impl FixPlan {
    /// Current text of a file, including changes earlier passes planned, or
    /// `None` when the file cannot be safely rewritten (recorded as skipped).
    fn read(&mut self, path: &Path) -> Result<Option<String>> {
        if let Some(text) = self.pending.get(path) {
            return Ok(Some(text.clone()));
        }
        if self.skipped.contains_key(path) {
            return Ok(None);
        }
        if let Some(reason) = unsafe_to_replace(path)? {
            self.skip(path, reason);
            return Ok(None);
        }
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        match String::from_utf8(bytes) {
            Ok(text) => Ok(Some(text)),
            Err(_) => {
                self.skip(path, "not valid UTF-8; rewriting it could corrupt it");
                Ok(None)
            }
        }
    }

    fn stage(&mut self, path: &Path, text: String) {
        self.pending.insert(path.to_path_buf(), text);
    }

    fn skip(&mut self, path: &Path, reason: impl Into<String>) {
        self.skipped.insert(path.to_path_buf(), reason.into());
    }

    fn inject_into_head(&mut self, path: &Path, tags: &[String]) -> Result<()> {
        let Some(raw_text) = self.read(path)? else {
            return Ok(());
        };
        let updated = inject_head_tags(&raw_text, tags);
        if updated != raw_text {
            self.stage(path, updated);
        }
        Ok(())
    }

    fn finish(self, dry_run: bool) -> Result<FixReport> {
        if !dry_run {
            for (path, text) in &self.pending {
                write_atomic(path, text).with_context(|| format!("writing {}", path.display()))?;
            }
        }
        Ok(FixReport {
            changed: self.pending.into_keys().collect(),
            skipped: self
                .skipped
                .into_iter()
                .map(|(path, reason)| SkippedFix { path, reason })
                .collect(),
        })
    }
}

/// Replacing a file by rename would detach a symlink or a hard link from what
/// it points at, so those are refused the same way `content --fix` refuses them.
fn unsafe_to_replace(path: &Path) -> Result<Option<&'static str>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    if !metadata.file_type().is_file() {
        return Ok(Some("not a regular file (symlinks are not rewritten)"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return Ok(Some("has multiple hard links"));
        }
    }
    Ok(None)
}

/// Write through a sibling temporary file and a rename, so a crash or a full
/// disk leaves either the old file or the new one, never a truncated page.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?
        .to_string_lossy();
    let temp_path = path.with_file_name(format!(
        ".{file_name}.aexeo-fix-{}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut temp = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        temp.write_all(text.as_bytes())?;
        match fs::metadata(path) {
            Ok(metadata) => temp.set_permissions(metadata.permissions())?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        temp.sync_all()?;
        fs::rename(&temp_path, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

/// HTML file for a route under `root`, in the conventional layouts: `index.html`
/// for the home route, then nested `<route>/index.html`, then flat `<route>.html`.
/// Routes come from JSON files on disk, so one that climbs out of `root` is refused.
fn html_path_for_route(root: &Path, route: &str) -> Result<Option<PathBuf>, String> {
    let relative = route.trim_start_matches('/');
    if !Path::new(relative)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(format!("route {route:?} points outside the site root"));
    }
    let candidates = if relative.is_empty() {
        vec![root.join("index.html")]
    } else {
        vec![
            root.join(relative).join("index.html"),
            root.join(format!("{relative}.html")),
        ]
    };
    Ok(candidates.into_iter().find(|path| path.exists()))
}

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

fn plan_html_metadata_fixes(plan: &mut FixPlan, site: &Site, config: &Config) -> Result<()> {
    for page in site.route_pages() {
        plan.inject_into_head(&page.path, &render_missing_head_tags(page, config))?;
    }
    Ok(())
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

fn plan_related_link_insertions(plan: &mut FixPlan, site: &Site, config: &Config) -> Result<()> {
    if !config.enable_link_autofix {
        return Ok(());
    }
    let suggestions = build_link_suggestions(site, config.link_suggestion_count);
    for (source_route, target_routes) in suggestions {
        let Some(source_page) = site.page(&source_route) else {
            continue;
        };
        let Some(raw_text) = plan.read(&source_page.path)? else {
            continue;
        };
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
            plan.stage(&source_page.path, updated);
        }
    }
    Ok(())
}

/// Inject schema-suggestions JSON-LD blocks into HTML pages, applying
/// per-type augmentation. Reads schema-suggestions.json from the same
/// root; for each route in the suggestions, finds the matching HTML
/// page (index.html / route/index.html / route.html) and adds any
/// suggested types the page does not already have. Existing
/// `<script type="application/ld+json">` blocks are inspected for
/// their `@type` and respected — Aexeo never overwrites or duplicates.
///
/// Silent no-op when the suggestions file is missing. Opt-in fix: only
/// runs when the caller explicitly enables it (the default `fix`
/// invocation does not write JSON-LD into pages).
fn plan_schema_injection_fixes(plan: &mut FixPlan, root: &Path) -> Result<()> {
    let suggestions_path = root.join("schema-suggestions.json");
    let Ok(suggestions_text) = fs::read_to_string(&suggestions_path) else {
        return Ok(());
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&suggestions_text) else {
        return Ok(());
    };
    let Some(suggestions) = payload.get("suggestions").and_then(|v| v.as_array()) else {
        return Ok(());
    };

    for entry in suggestions {
        let Some(route) = entry.get("route").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(types) = entry.get("types").and_then(|v| v.as_object()) else {
            continue;
        };
        let html_path = match html_path_for_route(root, route) {
            Ok(Some(path)) => path,
            Ok(None) => continue,
            Err(reason) => {
                plan.skip(&suggestions_path, reason);
                continue;
            }
        };
        let Some(raw_text) = plan.read(&html_path)? else {
            continue;
        };
        let existing_types = collect_existing_schema_types(&raw_text);
        let mut to_inject = Vec::new();
        for (type_name, body) in types {
            if existing_types.contains(type_name.as_str()) {
                continue;
            }
            to_inject.push(render_json_ld_script(body));
        }
        if to_inject.is_empty() {
            continue;
        }
        plan.inject_into_head(&html_path, &to_inject)?;
    }
    Ok(())
}

/// JSON-LD block for a page. serde_json leaves `<` as is, and suggestion text
/// comes from page content, so `</script>` in a value would end the block and
/// run what follows. JSON unicode escapes keep the value identical for parsers.
fn render_json_ld_script(body: &serde_json::Value) -> String {
    let body_text = serde_json::to_string(body)
        .unwrap_or_else(|_| "{}".to_string())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    format!("<script type=\"application/ld+json\">{body_text}</script>")
}

/// Scan an HTML string for the `@type` values of every existing
/// `<script type="application/ld+json">` block. We intentionally don't
/// fully parse the page or the JSON; we only need to know which types
/// are present so per-type augmentation can skip them. Both string and
/// array forms of `@type` are recognized (JSON-LD permits either).
fn collect_existing_schema_types(html: &str) -> std::collections::BTreeSet<String> {
    let mut found = std::collections::BTreeSet::new();
    let mut cursor = 0;
    let lower = html.to_ascii_lowercase();
    while let Some(start) = lower[cursor..].find("<script") {
        let absolute_start = cursor + start;
        let Some(open_end) = lower[absolute_start..].find('>') else {
            break;
        };
        let tag_end = absolute_start + open_end;
        let tag_open = &lower[absolute_start..=tag_end];
        if !tag_open.contains("application/ld+json") {
            cursor = tag_end + 1;
            continue;
        }
        let body_start = tag_end + 1;
        let Some(close_offset) = lower[body_start..].find("</script>") else {
            break;
        };
        let body_end = body_start + close_offset;
        let body = &html[body_start..body_end];
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body.trim()) {
            collect_at_types(&value, &mut found);
        }
        cursor = body_end;
    }
    found
}

fn collect_at_types(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(t) = map.get("@type") {
                match t {
                    serde_json::Value::String(s) => {
                        out.insert(s.clone());
                    }
                    serde_json::Value::Array(items) => {
                        for item in items {
                            if let serde_json::Value::String(s) = item {
                                out.insert(s.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
            for nested in map.values() {
                collect_at_types(nested, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_at_types(item, out);
            }
        }
        _ => {}
    }
}

/// Inject `<link rel="alternate" type="text/markdown" href="...">` tags into
/// every HTML page that has a markdown mirror declared in the bundle's
/// manifest.json. Idempotent: re-runs are no-ops once the tag is present.
///
/// Reads manifest.json from the same root the fix is run on. When no
/// manifest exists (i.e. the host hasn't generated a public-bundle there),
/// the fix is a silent no-op — the safer behavior than guessing at mirror
/// paths from filename conventions.
fn plan_discovery_link_fixes(plan: &mut FixPlan, root: &Path) -> Result<()> {
    let manifest_path = root.join("manifest.json");
    let Ok(manifest_text) = fs::read_to_string(&manifest_path) else {
        return Ok(());
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&manifest_text) else {
        return Ok(());
    };
    let Some(artifacts) = manifest.get("artifacts").and_then(|v| v.as_array()) else {
        return Ok(());
    };

    // Build a route → mirror-path map from the manifest. The manifest's
    // source_route is the leading "/" form ("/about", "/"); the matching
    // HTML file lives at root/about.html, root/about/index.html, or
    // root/index.html depending on the host's flat-vs-nested layout.
    let mut mirror_by_route: BTreeMap<String, String> = BTreeMap::new();
    for entry in artifacts {
        let Some(kind) = entry.get("kind").and_then(|v| v.as_str()) else {
            continue;
        };
        if kind != "markdown_mirror" {
            continue;
        }
        let Some(path) = entry.get("path").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(route) = entry.get("source_route").and_then(|v| v.as_str()) else {
            continue;
        };
        mirror_by_route.insert(route.to_string(), path.to_string());
    }
    if mirror_by_route.is_empty() {
        return Ok(());
    }

    for (route, mirror_path) in &mirror_by_route {
        let html_path = match html_path_for_route(root, route) {
            Ok(Some(path)) => path,
            Ok(None) => continue,
            Err(reason) => {
                plan.skip(&manifest_path, reason);
                continue;
            }
        };
        let Some(raw_text) = plan.read(&html_path)? else {
            continue;
        };
        // Idempotency: skip if the discovery link is already present.
        // Match on the href specifically rather than just the rel/type
        // string so a different mirror path doesn't get silently
        // duplicated. The mirror path is the canonical signal.
        let href = format!("/{mirror_path}");
        let needle = format!("href=\"{href}\"");
        if raw_text.contains(&needle) {
            continue;
        }
        let tag = format!(r#"<link rel="alternate" type="text/markdown" href="{href}">"#);
        plan.inject_into_head(&html_path, &[tag])?;
    }
    Ok(())
}

/// Caller-controlled toggles for fixes that mutate the host's HTML beyond
/// the minimum-viable metadata path. Each flag opts into one specific
/// content-bearing change, so hosts can gate them per release rather than
/// inheriting them all when the package version bumps.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixOptions {
    /// When true, inject schema-suggestions.json contents as JSON-LD
    /// blocks into matching HTML pages, applying per-type augmentation.
    /// Off by default because schema injection is content-bearing — the
    /// host is making claims about its own content; that should be
    /// deliberate.
    pub inject_schema: bool,
    /// When true, compute and report every change without writing any file.
    pub dry_run: bool,
}

pub fn apply_safe_fixes(root: &Path, config: &Config) -> Result<FixReport> {
    apply_safe_fixes_with_options(root, config, FixOptions::default())
}

/// `llms.txt` is only regenerated when it is known to be Aexeo's own output:
/// it carries the generated marker, it is blank, or it matches what an earlier
/// marker-less version rendered for this site. Anything else may be hand-written.
fn plan_llms_fix(plan: &mut FixPlan, site: &Site, root: &Path, config: &Config) -> Result<()> {
    let llms_path = root.join("llms.txt");
    if fs::symlink_metadata(&llms_path).is_err() {
        return Ok(());
    }
    let Some(current) = plan.read(&llms_path)? else {
        return Ok(());
    };
    let site_url = config.site_url.as_deref();
    let generated = render_llms_txt(site, site_url);
    if current == generated || current == render_bundle_llms_txt(site) {
        return Ok(());
    }
    let is_generated = current.trim().is_empty()
        || current
            .lines()
            .any(|line| line.trim() == LLMS_GENERATED_MARKER)
        || generated.strip_suffix(&format!("{LLMS_GENERATED_MARKER}\n")) == Some(current.as_str());
    if is_generated {
        plan.stage(&llms_path, generated);
    } else {
        plan.skip(
            &llms_path,
            format!(
                "no generated marker, so it may be hand-written; add the line `{LLMS_GENERATED_MARKER}` to let fix regenerate it"
            ),
        );
    }
    Ok(())
}

/// Create a missing `robots.txt`, or append the `Sitemap:` line to an existing
/// one. Existing directives are always kept.
fn plan_robots_fix(plan: &mut FixPlan, root: &Path, site_url: &str) -> Result<()> {
    let robots_path = root.join("robots.txt");
    if fs::symlink_metadata(&robots_path).is_err() {
        plan.stage(&robots_path, render_robots_txt(site_url));
        return Ok(());
    }
    let Some(text) = plan.read(&robots_path)? else {
        return Ok(());
    };
    if !text.to_ascii_lowercase().contains("sitemap:") {
        plan.stage(
            &robots_path,
            format!(
                "{}\nSitemap: {}/sitemap.xml\n",
                text.trim_end(),
                site_url.trim_end_matches('/')
            ),
        );
    }
    Ok(())
}

pub fn apply_safe_fixes_with_options(
    root: &Path,
    config: &Config,
    options: FixOptions,
) -> Result<FixReport> {
    let site = load_site(root)?;
    let mut plan = FixPlan::default();

    plan_llms_fix(&mut plan, &site, root, config)?;
    if let Some(site_url) = config.site_url.as_deref() {
        plan_robots_fix(&mut plan, root, site_url)?;
    }
    plan_html_metadata_fixes(&mut plan, &site, config)?;
    plan_related_link_insertions(&mut plan, &site, config)?;
    // Discovery-link injection runs against the manifest (if present) so
    // hosts who've generated a public-bundle into the same root get their
    // <link rel="alternate"> tags wired up automatically. Silent no-op
    // when no manifest is found — the safer behavior than guessing at
    // mirror paths from filename conventions.
    plan_discovery_link_fixes(&mut plan, root)?;
    if options.inject_schema {
        plan_schema_injection_fixes(&mut plan, root)?;
    }

    plan.finish(options.dry_run)
}

#[cfg(test)]
mod tests {
    use super::{FixOptions, apply_safe_fixes, apply_safe_fixes_with_options};
    use crate::config::Config;
    use crate::generate::LLMS_GENERATED_MARKER;
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
            &format!(
                "# Site\n\n## Key Facts\n- 9 features across 3 categories\n\n## Pages\n- [Home](index.html)\n{LLMS_GENERATED_MARKER}\n"
            ),
        );
        let config = Config {
            site_url: Some("https://example.com".to_string()),
            ..Config::default()
        };
        let changed = apply_safe_fixes(root, &config)?;
        assert!(!changed.changed.is_empty());
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

    #[test]
    fn injects_schema_with_per_type_augmentation_and_idempotency() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        // Page already has Article schema — that one type should be
        // skipped during injection. BreadcrumbList should still land.
        write(
            &root.join("guide.html"),
            r#"<html lang="en"><head>
                <title>Guide</title>
                <meta name="description" content="g">
                <link rel="canonical" href="https://example.com/guide">
                <script type="application/ld+json">{"@context":"https://schema.org","@type":"Article","headline":"Hand-authored"}</script>
            </head><body><h1>Guide</h1></body></html>"#,
        );
        write(
            &root.join("schema-suggestions.json"),
            r#"{
              "version": 1,
              "site_url": "https://example.com",
              "suggestions": [
                {
                  "route": "guide",
                  "types": {
                    "BreadcrumbList": {"@context": "https://schema.org", "@type": "BreadcrumbList", "itemListElement": []},
                    "Article": {"@context": "https://schema.org", "@type": "Article", "headline": "Aexeo-generated"}
                  }
                }
              ]
            }"#,
        );

        let config = Config::default();
        let options = FixOptions {
            inject_schema: true,
            ..FixOptions::default()
        };
        apply_safe_fixes_with_options(root, &config, options)?;

        let updated = fs::read_to_string(root.join("guide.html"))?;
        // Per-type augmentation: BreadcrumbList added, Article NOT
        // duplicated.
        assert!(updated.contains("\"BreadcrumbList\""));
        assert_eq!(
            updated.matches("\"Article\"").count(),
            1,
            "Article type must not be duplicated when host already authored one"
        );
        // The hand-authored Article must survive verbatim.
        assert!(updated.contains("Hand-authored"));
        assert!(!updated.contains("Aexeo-generated"));

        // Idempotency: a second run does not re-add the BreadcrumbList.
        apply_safe_fixes_with_options(root, &config, options)?;
        let twice = fs::read_to_string(root.join("guide.html"))?;
        assert_eq!(
            twice.matches("\"BreadcrumbList\"").count(),
            1,
            "schema injection must not duplicate types on second run"
        );
        Ok(())
    }

    #[test]
    fn injected_schema_cannot_close_its_script_block() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title></head><body></body></html>",
        );
        write(
            &root.join("schema-suggestions.json"),
            r#"{"suggestions":[{"route":"","types":{"WebSite":{"@type":"WebSite","name":"</script><script>alert(1)</script> & co"}}}]}"#,
        );

        apply_safe_fixes_with_options(
            root,
            &Config::default(),
            FixOptions {
                inject_schema: true,
                ..FixOptions::default()
            },
        )?;

        let html = fs::read_to_string(root.join("index.html"))?;
        assert_eq!(html.matches("</script>").count(), 1, "{html}");
        assert!(!html.contains("<script>"), "{html}");
        let start = html.find("ld+json\">").unwrap() + "ld+json\">".len();
        let end = html.find("</script>").unwrap();
        let value: serde_json::Value = serde_json::from_str(&html[start..end])?;
        assert_eq!(value["name"], "</script><script>alert(1)</script> & co");
        Ok(())
    }

    #[test]
    fn schema_injection_off_by_default() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("page.html"),
            r#"<html lang="en"><head><title>p</title><meta name="description" content="p"><link rel="canonical" href="https://example.com/page"></head><body><h1>p</h1></body></html>"#,
        );
        write(
            &root.join("schema-suggestions.json"),
            r#"{"version": 1, "site_url": "https://example.com", "suggestions": [
              {"route": "page", "types": {"BreadcrumbList": {"@context": "https://schema.org", "@type": "BreadcrumbList", "itemListElement": []}}}
            ]}"#,
        );

        let config = Config::default();
        // Default options (inject_schema=false) — schema-suggestions.json
        // is on disk but the fix should not write any schema into the
        // page. Content-bearing changes are opt-in.
        super::apply_safe_fixes(root, &config)?;
        let unchanged = fs::read_to_string(root.join("page.html"))?;
        assert!(!unchanged.contains("BreadcrumbList"));
        Ok(())
    }

    #[test]
    fn injects_discovery_links_for_manifest_listed_mirrors() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), &make_html_page("", ""));
        write(&root.join("about.html"), &make_html_page("about", ""));
        // Hand-craft a manifest matching the public-bundle output shape.
        write(
            &root.join("manifest.json"),
            r#"{
              "version": 1,
              "site_url": "https://example.com",
              "artifacts": [
                {"kind": "markdown_mirror", "path": "index.md.txt", "source_route": "/"},
                {"kind": "markdown_mirror", "path": "about.md.txt", "source_route": "/about"},
                {"kind": "sitemap", "path": "sitemap.xml"}
              ]
            }"#,
        );
        // Empty mirrors so the fix can find HTML and inject without
        // depending on real mirror generation; we're testing the
        // injection, not the bundle.
        write(&root.join("index.md.txt"), "");
        write(&root.join("about.md.txt"), "");

        let config = Config::default();
        apply_safe_fixes(root, &config)?;

        let home = fs::read_to_string(root.join("index.html"))?;
        assert!(
            home.contains(r#"href="/index.md.txt""#),
            "home should have the index mirror link"
        );
        let about = fs::read_to_string(root.join("about.html"))?;
        assert!(
            about.contains(r#"href="/about.md.txt""#),
            "about should have the about mirror link"
        );

        // Idempotency: a second run should not duplicate the link.
        apply_safe_fixes(root, &config)?;
        let home2 = fs::read_to_string(root.join("index.html"))?;
        assert_eq!(
            home2.matches(r#"href="/index.md.txt""#).count(),
            1,
            "discovery link must not be duplicated on second run"
        );
        Ok(())
    }

    const PAGE: &str = "<html lang=\"en\"><head><title>x</title><meta name=\"description\" content=\"y\"></head><body><h1>x</h1></body></html>";

    fn site_config() -> Config {
        Config {
            site_url: Some("https://example.com".to_string()),
            ..Config::default()
        }
    }

    fn snapshot(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fs::read_dir(root)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn hand_written_llms_is_kept_and_reported_as_skipped() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), PAGE);
        let hand_written =
            "# Acme\n\n> Hand-written summary.\n\n## Pages\n- [Home](/): start here\n";
        write(&root.join("llms.txt"), hand_written);

        let report = apply_safe_fixes(root, &site_config())?;

        assert_eq!(fs::read_to_string(root.join("llms.txt"))?, hand_written);
        assert!(!report.changed.contains(&root.join("llms.txt")));
        assert_eq!(report.skipped.len(), 1, "{report:?}");
        assert_eq!(report.skipped[0].path, root.join("llms.txt"));
        assert!(report.skipped[0].reason.contains("marker"));
        Ok(())
    }

    #[test]
    fn generated_llms_is_refreshed_when_routes_change() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), PAGE);
        let site = crate::site::load_site(root)?;
        write(
            &root.join("llms.txt"),
            &crate::generate::render_llms_txt(&site, None),
        );
        write(&root.join("pricing.html"), PAGE);

        let report = apply_safe_fixes(root, &site_config())?;

        assert!(report.skipped.is_empty(), "{report:?}");
        let refreshed = fs::read_to_string(root.join("llms.txt"))?;
        assert!(refreshed.contains("/pricing"), "{refreshed}");
        assert!(refreshed.contains(LLMS_GENERATED_MARKER));
        Ok(())
    }

    #[test]
    fn marker_less_generated_llms_gains_the_marker() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), PAGE);
        let site = crate::site::load_site(root)?;
        let current = crate::generate::render_llms_txt(&site, None);
        let legacy = current
            .strip_suffix(&format!("{LLMS_GENERATED_MARKER}\n"))
            .unwrap();
        write(&root.join("llms.txt"), legacy);

        let report = apply_safe_fixes(root, &site_config())?;

        assert!(report.skipped.is_empty(), "{report:?}");
        assert_eq!(fs::read_to_string(root.join("llms.txt"))?, current);
        Ok(())
    }

    #[test]
    fn current_public_bundle_llms_is_left_alone() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), PAGE);
        let site = crate::site::load_site(root)?;
        let bundle = crate::generate::render_bundle_llms_txt(&site);
        assert!(bundle.contains("## Markdown Mirrors"), "{bundle}");
        write(&root.join("llms.txt"), &bundle);

        let report = apply_safe_fixes(root, &site_config())?;

        assert!(report.skipped.is_empty(), "{report:?}");
        assert_eq!(fs::read_to_string(root.join("llms.txt"))?, bundle);
        Ok(())
    }

    #[test]
    fn non_utf8_robots_is_not_replaced_by_the_template() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), PAGE);
        let robots = b"User-agent: *\nDisallow: /caf\xe9\n".to_vec();
        fs::write(root.join("robots.txt"), &robots)?;

        let report = apply_safe_fixes(root, &site_config())?;

        assert_eq!(fs::read(root.join("robots.txt"))?, robots);
        assert_eq!(report.skipped[0].path, root.join("robots.txt"));
        Ok(())
    }

    #[test]
    fn existing_robots_keeps_its_directives_when_sitemap_is_added() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), PAGE);
        write(
            &root.join("robots.txt"),
            "User-agent: *\nDisallow: /private\n",
        );

        apply_safe_fixes(root, &site_config())?;

        assert_eq!(
            fs::read_to_string(root.join("robots.txt"))?,
            "User-agent: *\nDisallow: /private\nSitemap: https://example.com/sitemap.xml\n"
        );
        Ok(())
    }

    #[test]
    fn dry_run_lists_the_same_changes_as_a_real_run_and_writes_nothing() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("index.html"), PAGE);
        write(
            &root.join("schema-suggestions.json"),
            r#"{"suggestions":[{"route":"","types":{"WebSite":{"@type":"WebSite"}}}]}"#,
        );
        let before = snapshot(root);
        let config = site_config();
        let options = FixOptions {
            inject_schema: true,
            dry_run: true,
        };

        let planned = apply_safe_fixes_with_options(root, &config, options)?;

        assert_eq!(snapshot(root), before, "dry run must not touch the site");
        assert_eq!(
            planned.changed,
            vec![root.join("index.html"), root.join("robots.txt")]
        );
        let applied = apply_safe_fixes_with_options(
            root,
            &config,
            FixOptions {
                dry_run: false,
                ..options
            },
        )?;
        assert_eq!(applied.changed, planned.changed);
        assert!(fs::read_to_string(root.join("index.html"))?.contains("WebSite"));
        Ok(())
    }

    #[test]
    fn suggestion_route_cannot_write_outside_the_root() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().join("site");
        write(&root.join("index.html"), PAGE);
        let outside = temp_dir.path().join("outside.html");
        write(&outside, PAGE);
        write(
            &root.join("schema-suggestions.json"),
            r#"{"suggestions":[{"route":"../outside","types":{"WebSite":{"@type":"WebSite"}}}]}"#,
        );

        let report = apply_safe_fixes_with_options(
            &root,
            &site_config(),
            FixOptions {
                inject_schema: true,
                ..FixOptions::default()
            },
        )?;

        assert_eq!(fs::read_to_string(&outside)?, PAGE);
        assert_eq!(report.skipped[0].path, root.join("schema-suggestions.json"));
        Ok(())
    }

    #[test]
    fn atomic_write_leaves_no_temporary_file_behind() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("page.html");
        write(&path, "old");

        super::write_atomic(&path, "new")?;

        assert_eq!(fs::read_to_string(&path)?, "new");
        assert_eq!(fs::read_dir(temp_dir.path())?.count(), 1);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_file_permissions() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("page.html");
        write(&path, "old");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;

        super::write_atomic(&path, "new")?;

        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o640);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_and_hard_linked_pages_are_skipped_not_detached() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().join("site");
        write(&root.join("index.html"), PAGE);
        write(&root.join("robots.txt"), "User-agent: *\n");
        let robots_target = temp_dir.path().join("shared-robots.txt");
        fs::rename(root.join("robots.txt"), &robots_target)?;
        std::os::unix::fs::symlink(&robots_target, root.join("robots.txt"))?;
        let html_link = temp_dir.path().join("hard-link.html");
        fs::hard_link(root.join("index.html"), &html_link)?;

        let report = apply_safe_fixes(&root, &site_config())?;

        assert!(report.changed.is_empty(), "{report:?}");
        assert_eq!(report.skipped.len(), 2, "{report:?}");
        assert_eq!(fs::read_to_string(&robots_target)?, "User-agent: *\n");
        assert!(fs::symlink_metadata(root.join("robots.txt"))?.is_symlink());
        assert_eq!(fs::read_to_string(&html_link)?, PAGE);
        Ok(())
    }
}
