use aexeo_contracts::{Finding, FindingScope};
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::config::Config;
use crate::site::{Page, Site, decode_url_path};
use url::Url;

fn finding(
    rule_id: &str,
    message: String,
    suggestion: &str,
    page: &Page,
    line: usize,
    column: usize,
) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message,
        path: page.path.to_string_lossy().into_owned(),
        line,
        column,
        severity: "warning".to_string(),
        suggestion: Some(suggestion.to_string()),
        scope: FindingScope::Page,
    }
}

fn local_image_url_path(page: &Page, src: &str, site_url: Option<&str>) -> Option<String> {
    let source = src.trim();
    if source.is_empty() || source.starts_with("data:") || source.starts_with("blob:") {
        return None;
    }
    let expected_origin = configured_page_origin(page, site_url);
    if source.starts_with("//") {
        let base = page
            .canonical
            .as_deref()
            .or_else(|| page.meta_by_property.get("og:url").map(String::as_str))
            .or(site_url)?;
        let url = Url::parse(base.trim()).ok()?.join(source).ok()?;
        let origin = url.origin().ascii_serialization().to_ascii_lowercase();
        return (matches!(url.scheme(), "http" | "https")
            && expected_origin.as_deref() == Some(origin.as_str()))
        .then(|| url.path().to_string());
    }
    if let Ok(url) = Url::parse(source) {
        let origin = url.origin().ascii_serialization().to_ascii_lowercase();
        return (matches!(url.scheme(), "http" | "https")
            && expected_origin.as_deref() == Some(origin.as_str()))
        .then(|| url.path().to_string());
    }
    if source.contains("://") {
        return None;
    }
    Some(source.split(['?', '#']).next()?.trim().to_string())
}

fn local_image_filename(
    page: &Page,
    src: &str,
    site_url: Option<&str>,
) -> Option<(String, String)> {
    let path = local_image_url_path(page, src, site_url)?;
    let path = decode_url_path(&path)?;
    let filename = path.rsplit('/').next()?.trim();
    let (stem, extension) = filename.rsplit_once('.')?;
    if stem.is_empty() || extension.is_empty() {
        return None;
    }
    Some((stem.to_string(), extension.to_string()))
}

fn is_generic_filename(stem: &str) -> bool {
    let compact = stem
        .chars()
        .filter(|character| !matches!(character, '-' | '_' | ' '))
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if compact.chars().all(|character| character.is_ascii_digit()) {
        return true;
    }
    [
        "img",
        "image",
        "photo",
        "picture",
        "dsc",
        "pxl",
        "screenshot",
        "scan",
        "untitled",
    ]
    .iter()
    .any(|prefix| {
        compact.strip_prefix(prefix).is_some_and(|suffix| {
            suffix.is_empty() || suffix.chars().all(|character| character.is_ascii_digit())
        })
    })
}

fn is_lowercase_kebab_filename(stem: &str, extension: &str) -> bool {
    let valid_stem = !stem.is_empty()
        && !stem.starts_with('-')
        && !stem.ends_with('-')
        && !stem.contains("--")
        && stem.chars().all(|character| {
            character == '-'
                || character.is_lowercase()
                || (character.is_alphanumeric() && !character.is_uppercase())
        });
    let valid_extension = !extension.is_empty()
        && extension
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit());
    valid_stem && valid_extension
}

fn collect_filename_findings(page: &Page, config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    for image in &page.images {
        let mut seen_sources = std::collections::HashSet::new();
        for src in std::iter::once(image.src.as_str())
            .chain(image.asset_sources.iter().map(String::as_str))
        {
            if !seen_sources.insert(src) {
                continue;
            }
            let Some((stem, extension)) =
                local_image_filename(page, src, config.site_url.as_deref())
            else {
                continue;
            };
            let filename = format!("{stem}.{extension}");
            if is_generic_filename(&stem) {
                findings.push(finding(
                    "IMG001",
                    format!("image filename is generic: {filename}"),
                    "Rename it with a concise description of the image content.",
                    page,
                    image.line,
                    image.column,
                ));
            }
            if !is_lowercase_kebab_filename(&stem, &extension) {
                findings.push(finding(
                    "IMG002",
                    format!("image filename is not lowercase kebab-case: {filename}"),
                    "Use lowercase words separated by single hyphens; keep the existing URL stable or add a redirect when renaming.",
                    page,
                    image.line,
                    image.column,
                ));
            }
        }
    }
    findings
}

fn local_asset_relative_path(page: &Page, src: &str, site_url: Option<&str>) -> Option<PathBuf> {
    let path = local_image_url_path(page, src, site_url)?;
    let path = decode_url_path(&path)?;
    let candidate = if path.starts_with('/') {
        PathBuf::from(path.trim_start_matches('/'))
    } else {
        Path::new(&page.relative_path)
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(&path)
    };
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn collect_asset_findings(
    page: &Page,
    site: &Site,
    config: &Config,
) -> (Vec<Finding>, std::collections::HashSet<String>) {
    let mut findings = Vec::new();
    let mut missing_sources = std::collections::HashSet::new();
    let Ok(canonical_root) = fs::canonicalize(&site.root) else {
        return (findings, missing_sources);
    };
    for image in &page.images {
        let mut seen_sources = std::collections::HashSet::new();
        for src in std::iter::once(image.src.as_str())
            .chain(image.asset_sources.iter().map(String::as_str))
        {
            if !seen_sources.insert(src) {
                continue;
            }
            let Some(relative_path) =
                local_asset_relative_path(page, src, config.site_url.as_deref())
            else {
                continue;
            };
            let asset_path = site.root.join(relative_path);
            match fs::canonicalize(&asset_path) {
                Ok(canonical_asset) if canonical_asset.starts_with(&canonical_root) => {
                    let Ok(metadata) = fs::metadata(&canonical_asset) else {
                        continue;
                    };
                    if metadata.is_file() && metadata.len() > config.images.max_bytes {
                        findings.push(finding(
                            "IMG003",
                            format!(
                                "local image is {} bytes, above the configured {}-byte budget: {}",
                                metadata.len(),
                                config.images.max_bytes,
                                src
                            ),
                            "Compress or resize a copy and compare visual quality before replacing the published asset.",
                            page,
                            image.line,
                            image.column,
                        ));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    missing_sources.insert(src.to_string());
                    findings.push(finding(
                        "IMG004",
                        format!("local image asset does not exist: {}", src),
                        "Check the asset path and the build output; external and data URLs are not checked as local files.",
                        page,
                        image.line,
                        image.column,
                    ));
                }
                _ => {}
            }
        }
    }
    (findings, missing_sources)
}

fn collect_alt_findings(page: &Page, config: &Config) -> Vec<Finding> {
    let max_characters = config.images.max_alt_characters;
    page.images
        .iter()
        .filter_map(|image| {
            let alt = image.alt.as_deref()?;
            if image.is_marked_decorative() {
                return None;
            }
            let characters = alt.chars().count();
            if characters <= max_characters {
                return None;
            }
            Some(finding(
                "IMG006",
                format!(
                    "informative image alt text is {} characters, above the configured {}-character budget",
                    characters, max_characters
                ),
                "Keep the alt concise and specific; move longer explanations into nearby visible text or a caption.",
                page,
                image.line,
                image.column,
            ))
        })
        .collect()
}

fn collect_dimension_findings(page: &Page, config: &Config) -> Vec<Finding> {
    if !config.images.require_dimensions {
        return Vec::new();
    }
    page.images
        .iter()
        .filter(|image| {
            let valid = |value: Option<&String>| {
                value
                    .and_then(|value| value.parse::<u32>().ok())
                    .is_some_and(|value| value > 0)
            };
            !valid(image.width.as_ref()) || !valid(image.height.as_ref())
        })
        .map(|image| {
            finding(
                "IMG007",
                format!("image is missing valid width and height attributes: {}", image.src),
                "Set both intrinsic dimensions so browsers can reserve the image's aspect ratio before it loads.",
                page,
                image.line,
                image.column,
            )
        })
        .collect()
}

fn collect_caption_findings(page: &Page, config: &Config) -> Vec<Finding> {
    if !config.images.recommend_figure_captions {
        return Vec::new();
    }
    page.images
        .iter()
        .filter(|image| {
            image.inside_figure
                && image.figure_caption.is_none()
                && !image.is_marked_decorative()
        })
        .map(|image| {
            finding(
                "IMG008",
                format!("informative image in <figure> has no visible <figcaption>: {}", image.src),
                "Add a concise visible caption that explains why the image matters; keep purely decorative imagery uncaptioned.",
                page,
                image.line,
                image.column,
            )
        })
        .collect()
}

fn collect_format_findings(
    page: &Page,
    config: &Config,
    missing_sources: &std::collections::HashSet<String>,
) -> Vec<Finding> {
    if !config.images.prefer_modern_formats {
        return Vec::new();
    }
    page.images
        .iter()
        .filter_map(|image| {
            let has_available_modern_source = image.has_modern_source
                && (image.modern_sources.is_empty()
                    || image
                        .modern_sources
                        .iter()
                        .any(|source| !missing_sources.contains(source)));
            if has_available_modern_source {
                return None;
            }
            let (_, extension) =
                local_image_filename(page, &image.src, config.site_url.as_deref())?;
            if !matches!(extension.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png") {
                return None;
            }
            Some(finding(
                "IMG005",
                format!("legacy raster image has no WebP/AVIF picture source: {}", image.src),
                "Consider adding a WebP or AVIF <source> with the current image as a compatibility fallback; compare visual quality before switching.",
                page,
                image.line,
                image.column,
            ))
        })
        .collect()
}

fn sitemap_image_url_keys(
    site: &Site,
    config: &Config,
) -> Result<std::collections::BTreeSet<String>, String> {
    let allowed_origin = configured_site_origin(site, config);
    let image_urls = if site.deployment_model == crate::site::DeploymentModel::RuntimeSnapshot {
        let Some(text) = site.sitemap_text.as_deref() else {
            return Err("the runtime snapshot did not capture sitemap.xml".to_string());
        };
        crate::site::sitemap_image_urls_from_text(text).ok_or_else(|| {
            "the runtime snapshot does not contain a directly captured sitemap urlset; nested sitemap indexes cannot be fetched from the snapshot".to_string()
        })?
    } else {
        crate::site::sitemap_image_urls(&site.root, allowed_origin.as_deref())
            .map_err(|error| error.to_string())?
    };

    Ok(image_urls
        .into_iter()
        .filter_map(|value| Url::parse(value.trim()).ok())
        .filter_map(absolute_image_object_key)
        .collect())
}

fn local_asset_absolute_url_key(
    page: &Page,
    path: &Path,
    site_url: Option<&str>,
    trailing_slash: bool,
) -> Option<String> {
    let origin = configured_page_origin(page, site_url)?;
    let mut url = Url::parse(&format!("{origin}/")).ok()?;
    {
        let mut segments = url.path_segments_mut().ok()?;
        segments.pop_if_empty();
        for component in path.components() {
            let Component::Normal(value) = component else {
                return None;
            };
            segments.push(value.to_str()?);
        }
        if trailing_slash {
            segments.push("");
        }
    }
    absolute_image_object_key(url)
}

fn collect_image_sitemap_findings(
    page: &Page,
    config: &Config,
    image_sitemap_url_keys: &std::collections::BTreeSet<String>,
) -> (Vec<Finding>, bool) {
    if !config.images.require_image_sitemap {
        return (Vec::new(), false);
    }
    let mut origin_unavailable = false;
    let findings = page
        .images
        .iter()
        .filter_map(|image| {
            if image.is_marked_decorative() {
                return None;
            }
            let local_path = local_asset_relative_path(page, &image.src, config.site_url.as_deref())?;
            let Some(image_url_key) = local_asset_absolute_url_key(
                page,
                &local_path,
                config.site_url.as_deref(),
                has_trailing_url_path_slash(&image.src),
            ) else {
                origin_unavailable = true;
                return None;
            };
            if image_sitemap_url_keys.contains(&image_url_key) {
                return None;
            }
            Some(finding(
                "IMG009",
                format!("informative local image is missing from the image sitemap: {}", image.src),
                "Add this image under an <image:image> entry for its page when it is important for image discovery.",
                page,
                image.line,
                image.column,
            ))
        })
        .collect();
    (findings, origin_unavailable)
}

fn image_sitemap_unavailable_finding(reason: &str) -> Finding {
    Finding {
        rule_id: "IMG009".to_string(),
        message: format!("image sitemap coverage cannot be checked: {reason}"),
        path: "sitemap.xml".to_string(),
        line: 1,
        column: 1,
        severity: "warning".to_string(),
        suggestion: Some(
            "Provide a captured sitemap urlset and a canonical page URL or configured site_url so image URL origins can be compared safely."
                .to_string(),
        ),
        scope: FindingScope::Sitewide,
    }
}

fn page_origin(page: &Page) -> Option<String> {
    let origin = |value: &str| {
        let url = Url::parse(value.trim()).ok()?;
        matches!(url.scheme(), "http" | "https")
            .then(|| url.origin().ascii_serialization().to_ascii_lowercase())
    };
    page.canonical.as_deref().and_then(origin).or_else(|| {
        page.meta_by_property
            .get("og:url")
            .and_then(|value| origin(value))
    })
}

fn configured_page_origin(page: &Page, site_url: Option<&str>) -> Option<String> {
    page_origin(page).or_else(|| {
        let url = Url::parse(site_url?.trim()).ok()?;
        matches!(url.scheme(), "http" | "https")
            .then(|| url.origin().ascii_serialization().to_ascii_lowercase())
    })
}

fn configured_site_origin(site: &Site, config: &Config) -> Option<String> {
    let configured = config.site_url.as_deref().and_then(|value| {
        let url = Url::parse(value.trim()).ok()?;
        matches!(url.scheme(), "http" | "https")
            .then(|| url.origin().ascii_serialization().to_ascii_lowercase())
    });
    configured.or_else(|| site.route_pages().find_map(page_origin))
}

fn local_image_object_key(path: &Path, trailing_slash: bool) -> String {
    format!(
        "local:{}{}",
        path.to_string_lossy().replace('\\', "/"),
        if trailing_slash { "/" } else { "" }
    )
}

fn has_trailing_url_path_slash(value: &str) -> bool {
    value
        .trim()
        .split(['?', '#'])
        .next()
        .and_then(decode_url_path)
        .is_some_and(|path| path.ends_with('/'))
}

fn absolute_image_object_key(mut url: Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    let origin = url.origin().ascii_serialization().to_ascii_lowercase();
    Some(format!("url:{origin}{}", url.path()))
}

fn image_object_url_key(page: &Page, value: &str, site_url: Option<&str>) -> Option<String> {
    let value = value.trim();
    if let Some(path) = local_asset_relative_path(page, value, site_url) {
        return Some(local_image_object_key(
            &path,
            has_trailing_url_path_slash(value),
        ));
    }

    let url = if value.starts_with("//") {
        let canonical = page
            .canonical
            .as_deref()
            .or_else(|| page.meta_by_property.get("og:url").map(String::as_str))
            .or(site_url)?;
        Url::parse(canonical.trim()).ok()?.join(value).ok()?
    } else {
        Url::parse(value).ok()?
    };
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }

    let origin = url.origin().ascii_serialization().to_ascii_lowercase();
    let local_path = if configured_page_origin(page, site_url).as_deref() == Some(origin.as_str()) {
        local_asset_relative_path(page, url.path(), site_url)
    } else {
        None
    };
    if let Some(path) = local_path {
        return Some(local_image_object_key(
            &path,
            has_trailing_url_path_slash(url.path()),
        ));
    }
    absolute_image_object_key(url)
}

fn has_nonempty_schema_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => !text.trim().is_empty(),
        serde_json::Value::Array(values) => values.iter().any(has_nonempty_schema_value),
        serde_json::Value::Object(properties) => [
            "name",
            "url",
            "@id",
            "identifier",
            "givenName",
            "familyName",
        ]
        .iter()
        .any(|key| properties.get(*key).is_some_and(has_nonempty_schema_value)),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            false
        }
    }
}

fn is_image_object(value: &serde_json::Value) -> bool {
    let Some(type_value) = value.get("@type") else {
        return false;
    };
    let is_type = |value: &serde_json::Value| {
        value.as_str().is_some_and(|name| {
            name.rsplit(['/', '#', ':'])
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("ImageObject"))
        })
    };
    match type_value {
        serde_json::Value::String(_) => is_type(type_value),
        serde_json::Value::Array(values) => values.iter().any(is_type),
        _ => false,
    }
}

fn collect_image_objects<'a>(
    value: &'a serde_json::Value,
    output: &mut Vec<&'a serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(properties) => {
            if is_image_object(value) {
                output.push(value);
            }
            for child in properties.values() {
                collect_image_objects(child, output);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                collect_image_objects(child, output);
            }
        }
        _ => {}
    }
}

fn image_object_metadata(
    page: &Page,
    site_url: Option<&str>,
) -> std::collections::BTreeMap<String, (bool, bool)> {
    let mut metadata = std::collections::BTreeMap::new();
    for block in &page.json_ld_blocks {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&block.raw) else {
            continue;
        };
        let mut objects = Vec::new();
        collect_image_objects(&value, &mut objects);
        for object in objects {
            let Some(content_url) = object
                .get("contentUrl")
                .or_else(|| object.get("url"))
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let Some(key) = image_object_url_key(page, content_url, site_url) else {
                continue;
            };
            let entry = metadata.entry(key).or_insert((false, false));
            entry.0 |= object.get("creator").is_some_and(has_nonempty_schema_value);
            entry.1 |= object.get("license").is_some_and(has_nonempty_schema_value);
        }
    }
    metadata
}

fn collect_image_object_findings(page: &Page, config: &Config) -> Vec<Finding> {
    if !config.images.require_image_object_metadata {
        return Vec::new();
    }
    let metadata = image_object_metadata(page, config.site_url.as_deref());
    page.images
        .iter()
        .filter(|image| !image.is_marked_decorative())
        .filter_map(|image| {
            let key = image_object_url_key(page, &image.src, config.site_url.as_deref())?;
            let (has_creator, has_license) = metadata.get(&key).copied().unwrap_or_default();
            if has_creator && has_license {
                return None;
            }
            let mut missing = Vec::new();
            if !metadata.contains_key(&key) {
                missing.push("a matching ImageObject with contentUrl");
            }
            if !has_creator {
                missing.push("creator");
            }
            if !has_license {
                missing.push("license");
            }
            Some(finding(
                "IMG010",
                format!(
                    "informative image is missing {} metadata: {}",
                    missing.join(" and "),
                    image.src
                ),
                "Add a matching ImageObject to this page's structured data with the image contentUrl, creator, and license.",
                page,
                image.line,
                image.column,
            ))
        })
        .collect()
}

pub fn run_image_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let mut sitemap_unavailable = None;
    let image_sitemap_url_keys = if config.images.require_image_sitemap {
        match sitemap_image_url_keys(site, config) {
            Ok(keys) => Some(keys),
            Err(reason) => {
                sitemap_unavailable = Some(reason);
                None
            }
        }
    } else {
        None
    };
    let mut findings = Vec::new();
    let mut origin_unavailable = false;
    for page in site.route_pages() {
        findings.extend(collect_filename_findings(page, config));
        let (asset_findings, missing_sources) = collect_asset_findings(page, site, config);
        findings.extend(asset_findings);
        findings.extend(collect_format_findings(page, config, &missing_sources));
        findings.extend(collect_alt_findings(page, config));
        findings.extend(collect_dimension_findings(page, config));
        findings.extend(collect_caption_findings(page, config));
        if let Some(image_sitemap_url_keys) = image_sitemap_url_keys.as_ref() {
            let (sitemap_findings, missing_origin) =
                collect_image_sitemap_findings(page, config, image_sitemap_url_keys);
            findings.extend(sitemap_findings);
            origin_unavailable |= missing_origin;
        }
        findings.extend(collect_image_object_findings(page, config));
    }
    if let Some(reason) = sitemap_unavailable {
        findings.push(image_sitemap_unavailable_finding(&reason));
    }
    if origin_unavailable {
        findings.push(image_sitemap_unavailable_finding(
            "one or more pages have local images but no canonical URL or configured site_url",
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::{is_generic_filename, is_lowercase_kebab_filename, run_image_rules};
    use crate::config::Config;
    use crate::site::{
        DeploymentModel, SiteArtifacts, SiteBuildInput, build_page_from_source,
        build_site_from_parts, load_site,
    };
    use aexeo_contracts::FindingScope;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn requires_image_object_license_and_creator_only_when_configured() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/desk-setup.webp"), b"image").unwrap();
        fs::write(root.join("images/office-chair.webp"), b"image").unwrap();
        fs::write(
            root.join("index.html"),
            r#"<html><head><link rel="canonical" href="https://example.com/"></head><body><img src="/images/desk-setup.webp" alt="Desk setup" width="640" height="480"><img src="/images/office-chair.webp" alt="Office chair" width="640" height="480"><script type="application/ld+json">[{"@context":"https://schema.org","@type":"ImageObject","contentUrl":"https://example.com/images/desk-setup.webp","creator":{"@type":"Person","name":"A. Photographer"},"license":"https://creativecommons.org/licenses/by/4.0/"},{"@type":"ImageObject","contentUrl":"https://example.com/images/unrelated.webp","creator":"A. Photographer","license":"https://example.com/license"}]</script></body></html>"#,
        )
        .unwrap();
        let site = load_site(root).unwrap();
        let mut config = Config::default();

        let default_findings = run_image_rules(&site, &config);
        assert!(
            !default_findings
                .iter()
                .any(|finding| finding.rule_id == "IMG010")
        );

        config.images.require_image_object_metadata = true;
        let findings = run_image_rules(&site, &config);
        let metadata_findings = findings
            .iter()
            .filter(|finding| finding.rule_id == "IMG010")
            .collect::<Vec<_>>();

        assert_eq!(metadata_findings.len(), 1);
        assert!(metadata_findings[0].message.contains("office-chair.webp"));
    }

    #[test]
    fn image_object_metadata_does_not_match_a_different_absolute_origin() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::write(
            root.join("index.html"),
            r#"<html><head><link rel="canonical" href="https://example.com/"></head><body><img src="https://cdn-a.example/images/desk-setup.webp" alt="Desk setup" width="640" height="480"><script type="application/ld+json">{"@type":"ImageObject","contentUrl":"https://cdn-b.example/images/desk-setup.webp","creator":"A. Photographer","license":"https://example.com/license"}</script></body></html>"#,
        )
        .unwrap();
        let site = load_site(root).unwrap();
        let mut config = Config::default();
        config.images.require_image_object_metadata = true;

        let findings = run_image_rules(&site, &config);

        assert!(findings.iter().any(|finding| finding.rule_id == "IMG010"));
    }

    #[test]
    fn image_object_metadata_does_not_match_distinct_trailing_slash_paths() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::write(
            root.join("index.html"),
            r#"<html><body><img src="https://cdn.example/photo.webp/" alt="Photo" width="640" height="480"><script type="application/ld+json">{"@type":"ImageObject","contentUrl":"https://cdn.example/photo.webp","creator":"A. Photographer","license":"https://example.com/license"}</script></body></html>"#,
        )
        .unwrap();
        let site = load_site(root).unwrap();
        let mut config = Config::default();
        config.images.require_image_object_metadata = true;

        let findings = run_image_rules(&site, &config);

        assert!(findings.iter().any(|finding| finding.rule_id == "IMG010"));
    }

    #[test]
    fn image_object_metadata_does_not_match_trailing_slash_same_origin_paths() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::write(
            root.join("index.html"),
            r#"<html><head><link rel="canonical" href="https://example.com/"></head><body><img src="https://example.com/images/photo.webp/" alt="Photo" width="640" height="480"><script type="application/ld+json">{"@type":"ImageObject","contentUrl":"https://example.com/images/photo.webp","creator":"A. Photographer","license":"https://example.com/license"}</script></body></html>"#,
        )
        .unwrap();
        let site = load_site(root).unwrap();
        let mut config = Config::default();
        config.images.require_image_object_metadata = true;

        let findings = run_image_rules(&site, &config);

        assert!(findings.iter().any(|finding| finding.rule_id == "IMG010"));
    }

    #[test]
    fn flags_local_images_missing_from_an_opted_in_image_sitemap() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/desk-setup.webp"), b"image").unwrap();
        fs::write(
            root.join("index.html"),
            "<html><body><img src=\"/images/desk-setup.webp\" alt=\"Desk setup\" width=\"640\" height=\"480\"></body></html>",
        )
        .unwrap();
        fs::write(
            root.join("sitemap.xml"),
            "<urlset><url><loc>https://example.com/</loc></url></urlset>",
        )
        .unwrap();
        let site = load_site(root).unwrap();
        let mut config = Config::default();
        config.images.require_image_sitemap = true;
        config.site_url = Some("https://example.com/".to_string());

        let findings = run_image_rules(&site, &config);

        assert!(findings.iter().any(|finding| finding.rule_id == "IMG009"));

        fs::write(
            root.join("sitemap.xml"),
            "<urlset xmlns:img=\"http://www.google.com/schemas/sitemap-image/1.1\"><url><loc>https://example.com/</loc><img:image><img:loc>https://example.com/images/desk-setup.webp</img:loc></img:image></url></urlset>",
        )
        .unwrap();
        let covered_findings = run_image_rules(&site, &config);
        assert!(
            !covered_findings
                .iter()
                .any(|finding| finding.rule_id == "IMG009")
        );

        fs::write(
            root.join("sitemap.xml"),
            "<s:sitemapindex xmlns:s=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><s:sitemap><s:loc>https://example.com/image-sitemap.xml</s:loc></s:sitemap></s:sitemapindex>",
        )
        .unwrap();
        fs::write(
            root.join("image-sitemap.xml"),
            "<s:urlset xmlns:s=\"http://www.sitemaps.org/schemas/sitemap/0.9\" xmlns:img=\"http://www.google.com/schemas/sitemap-image/1.1\"><s:url><s:loc>https://example.com/</s:loc><img:image><img:loc>https://example.com/images/desk-setup.webp</img:loc></img:image></s:url></s:urlset>",
        )
        .unwrap();
        let indexed_findings = run_image_rules(&site, &config);
        assert!(
            !indexed_findings
                .iter()
                .any(|finding| finding.rule_id == "IMG009")
        );

        fs::create_dir_all(root.join("sitemaps")).unwrap();
        fs::write(
            root.join("sitemap.xml"),
            "<sitemapindex><sitemap><loc>https://example.com/sitemaps/images.xml</loc></sitemap></sitemapindex>",
        )
        .unwrap();
        fs::write(
            root.join("sitemaps/images.xml"),
            "<urlset xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\"><url><loc>https://example.com/</loc><image:image><image:loc>https://example.com/images/desk-setup.webp</image:loc></image:image></url></urlset>",
        )
        .unwrap();
        let subdirectory_findings = run_image_rules(&site, &config);
        assert!(
            !subdirectory_findings
                .iter()
                .any(|finding| finding.rule_id == "IMG009")
        );

        fs::write(
            root.join("sitemap.xml"),
            "<sitemapindex><sitemap><loc>https://example.com/sitemaps/image-map</loc></sitemap></sitemapindex>",
        )
        .unwrap();
        fs::write(
            root.join("sitemaps/image-map"),
            "<urlset xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\"><url><loc>https://example.com/</loc><image:image><image:loc>https://example.com/images/desk-setup.webp</image:loc></image:image></url></urlset>",
        )
        .unwrap();
        let suffixless_findings = run_image_rules(&site, &config);
        assert!(
            !suffixless_findings
                .iter()
                .any(|finding| finding.rule_id == "IMG009")
        );
    }

    #[cfg(unix)]
    #[test]
    fn ignores_sitemap_symlinks_outside_site_root_when_image_sitemap_is_required() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().join("site");
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/desk-setup.webp"), b"image").unwrap();
        fs::write(
            root.join("index.html"),
            "<html><body><img src=\"/images/desk-setup.webp\" alt=\"Desk setup\" width=\"640\" height=\"480\"></body></html>",
        )
        .unwrap();
        let outside = temp_dir.path().join("outside-sitemap.xml");
        fs::write(
            &outside,
            "<urlset xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\"><url><loc>https://example.com/</loc><image:image><image:loc>https://example.com/images/desk-setup.webp</image:loc></image:image></url></urlset>",
        )
        .unwrap();
        symlink(&outside, root.join("sitemap.xml")).unwrap();
        let site = load_site(&root).unwrap();
        let mut config = Config::default();
        config.images.require_image_sitemap = true;
        config.site_url = Some("https://example.com/".to_string());

        let top_level_findings = run_image_rules(&site, &config);
        assert!(
            top_level_findings
                .iter()
                .any(|finding| finding.rule_id == "IMG009")
        );

        fs::remove_file(root.join("sitemap.xml")).unwrap();
        fs::write(
            root.join("sitemap.xml"),
            "<sitemapindex><sitemap><loc>https://example.com/private.xml</loc></sitemap></sitemapindex>",
        )
        .unwrap();
        symlink(&outside, root.join("private.xml")).unwrap();
        let nested_findings = run_image_rules(&site, &config);
        assert!(
            nested_findings
                .iter()
                .any(|finding| finding.rule_id == "IMG009")
        );
    }

    #[test]
    fn resolves_percent_encoded_local_image_paths_before_checking_assets_and_names() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/mañana-nube.jpg"), b"image").unwrap();
        fs::write(
            root.join("index.html"),
            "<html><body><img src=\"/images/ma%C3%B1ana-nube.jpg\" alt=\"Nube de la mañana\" width=\"640\" height=\"480\"></body></html>",
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());

        assert!(!findings.iter().any(|finding| finding.rule_id == "IMG004"));
        assert!(!findings.iter().any(|finding| finding.rule_id == "IMG002"));
    }

    #[test]
    fn flags_missing_local_image_assets() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::write(
            root.join("index.html"),
            "<html><body><img src=\"/images/desk-setup.webp\" alt=\"Desk setup\" width=\"640\" height=\"480\"></body></html>",
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());

        assert!(findings.iter().any(|finding| finding.rule_id == "IMG004"));
    }

    #[test]
    fn checks_absolute_same_origin_images_as_local_assets() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/img-123.JPG"), vec![b'x'; 100_001]).unwrap();
        fs::write(
            root.join("index.html"),
            "<html><head><link rel=\"canonical\" href=\"https://example.com/\"></head><body><img src=\"https://example.com/images/img-123.JPG\" alt=\"Photo\" width=\"640\" height=\"480\"><img src=\"https://example.com/images/missing-photo.webp\" alt=\"Missing photo\" width=\"640\" height=\"480\"></body></html>",
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());
        let rule_ids = findings
            .iter()
            .map(|finding| finding.rule_id.as_str())
            .collect::<Vec<_>>();

        assert!(rule_ids.contains(&"IMG001"));
        assert!(rule_ids.contains(&"IMG002"));
        assert!(rule_ids.contains(&"IMG003"));
        assert!(rule_ids.contains(&"IMG004"));
        assert!(rule_ids.contains(&"IMG005"));
    }

    #[test]
    fn image_sitemap_does_not_match_another_origin_with_the_same_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/chair.webp"), b"image").unwrap();
        fs::write(
            root.join("index.html"),
            "<html><head><link rel=\"canonical\" href=\"https://example.com/\"></head><body><img src=\"/images/chair.webp\" alt=\"Chair\" width=\"640\" height=\"480\"></body></html>",
        )
        .unwrap();
        fs::write(
            root.join("sitemap.xml"),
            "<urlset xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\"><url><loc>https://example.com/</loc><image:image><image:loc>https://other.invalid/images/chair.webp</image:loc></image:image></url></urlset>",
        )
        .unwrap();
        let site = load_site(root).unwrap();
        let mut config = Config::default();
        config.images.require_image_sitemap = true;
        config.site_url = Some("https://example.com/".to_string());

        let findings = run_image_rules(&site, &config);

        assert!(findings.iter().any(|finding| finding.rule_id == "IMG009"));
    }

    #[test]
    fn does_not_follow_nested_image_sitemaps_from_another_origin() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::create_dir_all(root.join("sitemaps")).unwrap();
        fs::write(root.join("images/chair.webp"), b"image").unwrap();
        fs::write(
            root.join("index.html"),
            "<html><head><link rel=\"canonical\" href=\"https://example.com/\"></head><body><img src=\"/images/chair.webp\" alt=\"Chair\" width=\"640\" height=\"480\"></body></html>",
        )
        .unwrap();
        fs::write(
            root.join("sitemap.xml"),
            "<sitemapindex><sitemap><loc>https://evil.example/sitemaps/images</loc></sitemap></sitemapindex>",
        )
        .unwrap();
        fs::write(
            root.join("sitemaps/images"),
            "<urlset xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\"><url><loc>https://example.com/</loc><image:image><image:loc>https://example.com/images/chair.webp</image:loc></image:image></url></urlset>",
        )
        .unwrap();
        let site = load_site(root).unwrap();
        let mut config = Config::default();
        config.images.require_image_sitemap = true;
        config.site_url = Some("https://example.com/".to_string());

        let findings = run_image_rules(&site, &config);

        assert!(findings.iter().any(|finding| {
            finding.rule_id == "IMG009"
                && finding.scope == FindingScope::Sitewide
                && finding.message.contains("cannot be checked")
        }));
    }

    #[test]
    fn image_sitemap_matches_percent_encoded_unicode_paths() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/mañana.webp"), b"image").unwrap();
        fs::write(
            root.join("index.html"),
            "<html><head><link rel=\"canonical\" href=\"https://example.com/\"></head><body><img src=\"/images/mañana.webp\" alt=\"Mañana\" width=\"640\" height=\"480\"></body></html>",
        )
        .unwrap();
        fs::write(
            root.join("sitemap.xml"),
            "<urlset xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\"><url><loc>https://example.com/</loc><image:image><image:loc>https://example.com/images/ma%C3%B1ana.webp</image:loc></image:image></url></urlset>",
        )
        .unwrap();
        let site = load_site(root).unwrap();
        let mut config = Config::default();
        config.images.require_image_sitemap = true;
        config.site_url = Some("https://example.com/".to_string());

        let findings = run_image_rules(&site, &config);

        assert!(!findings.iter().any(|finding| finding.rule_id == "IMG009"));
    }

    #[test]
    fn runtime_snapshot_uses_its_captured_image_sitemap() {
        let page = build_page_from_source(
            PathBuf::from("crawl/index.html"),
            "index.html".to_string(),
            "<html><head><link rel=\"canonical\" href=\"https://example.com/\"></head><body><img src=\"/images/chair.webp\" alt=\"Chair\" width=\"640\" height=\"480\"></body></html>".to_string(),
            BTreeMap::new(),
        );
        let site = build_site_from_parts(SiteBuildInput {
            root: PathBuf::from("crawl"),
            pages: vec![page],
            artifacts: SiteArtifacts {
                llms_text: None,
                robots_text: None,
                sitemap_text: Some("<urlset xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\"><url><loc>https://example.com/</loc><image:image><image:loc>https://example.com/images/chair.webp</image:loc></image:image></url></urlset>".to_string()),
            },
            deployment_model: DeploymentModel::RuntimeSnapshot,
            deployment_markers: vec!["runtime crawl snapshot".to_string()],
            crawl_meta: None,
        })
        .unwrap();
        let mut config = Config::default();
        config.images.require_image_sitemap = true;
        config.site_url = Some("https://example.com/".to_string());

        let findings = run_image_rules(&site, &config);

        assert!(!findings.iter().any(|finding| finding.rule_id == "IMG009"));

        let index_site = build_site_from_parts(SiteBuildInput {
            root: PathBuf::from("crawl"),
            pages: site.pages.clone(),
            artifacts: SiteArtifacts {
                llms_text: None,
                robots_text: None,
                sitemap_text: Some(
                    "<sitemapindex><sitemap><loc>https://example.com/images.xml</loc></sitemap></sitemapindex>"
                        .to_string(),
                ),
            },
            deployment_model: DeploymentModel::RuntimeSnapshot,
            deployment_markers: vec!["runtime crawl snapshot".to_string()],
            crawl_meta: None,
        })
        .unwrap();
        let index_findings = run_image_rules(&index_site, &config);
        assert!(index_findings.iter().any(|finding| {
            finding.rule_id == "IMG009"
                && finding.scope == FindingScope::Sitewide
                && finding.message.contains("cannot be checked")
        }));
    }

    #[test]
    fn recommends_a_visible_caption_for_informative_images_in_figures() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/desk-setup.webp"), b"image").unwrap();
        fs::write(
            root.join("index.html"),
            "<html><body><figure><img src=\"/images/desk-setup.webp\" alt=\"Desk with chair and monitor\" width=\"640\" height=\"480\"></figure></body></html>",
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());

        assert!(findings.iter().any(|finding| finding.rule_id == "IMG008"));

        fs::write(
            root.join("index.html"),
            "<html><body><figure><img src=\"/images/desk-setup.webp\" alt=\"Desk with chair and monitor\" width=\"640\" height=\"480\"><figcaption hidden>Hidden caption</figcaption></figure></body></html>",
        )
        .unwrap();
        let site_with_hidden_caption = load_site(root).unwrap();
        let hidden_caption_findings =
            run_image_rules(&site_with_hidden_caption, &Config::default());
        assert!(
            hidden_caption_findings
                .iter()
                .any(|finding| finding.rule_id == "IMG008")
        );

        fs::write(
            root.join("index.html"),
            "<html><body><figure><img src=\"/images/desk-setup.webp\" alt=\"Desk with chair and monitor\" width=\"640\" height=\"480\"><figcaption style=\"display: none\">Invisible caption</figcaption></figure></body></html>",
        )
        .unwrap();
        let site_with_invisible_caption = load_site(root).unwrap();
        let invisible_caption_findings =
            run_image_rules(&site_with_invisible_caption, &Config::default());
        assert!(
            invisible_caption_findings
                .iter()
                .any(|finding| finding.rule_id == "IMG008")
        );

        fs::write(
            root.join("index.html"),
            "<html><body><figure><img src=\"/images/desk-setup.webp\" alt=\"Desk with chair and monitor\" width=\"640\" height=\"480\"><figcaption><span hidden>Invisible nested caption</span></figcaption></figure></body></html>",
        )
        .unwrap();
        let site_with_hidden_descendant = load_site(root).unwrap();
        let hidden_descendant_findings =
            run_image_rules(&site_with_hidden_descendant, &Config::default());
        assert!(
            hidden_descendant_findings
                .iter()
                .any(|finding| finding.rule_id == "IMG008")
        );

        fs::write(
            root.join("index.html"),
            "<html><body><figure><img src=\"/images/desk-setup.webp\" alt=\"Desk with chair and monitor\" width=\"640\" height=\"480\"><figcaption><span hidden>Invisible words</span><span>Visible desk caption</span></figcaption></figure></body></html>",
        )
        .unwrap();
        let site_with_visible_descendant = load_site(root).unwrap();
        let visible_descendant_findings =
            run_image_rules(&site_with_visible_descendant, &Config::default());
        assert!(
            !visible_descendant_findings
                .iter()
                .any(|finding| finding.rule_id == "IMG008")
        );
    }

    #[test]
    fn warns_when_image_intrinsic_dimensions_are_missing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/office-chair.webp"), b"image").unwrap();
        fs::write(
            root.join("index.html"),
            "<html><body><img src=\"/images/office-chair.webp\" alt=\"Office chair\" width=\"640\"></body></html>",
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());

        assert!(findings.iter().any(|finding| finding.rule_id == "IMG007"));
    }

    #[test]
    fn warns_when_informative_alt_text_exceeds_the_default_character_budget() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/office-chair.webp"), b"image").unwrap();
        let alt = "a".repeat(126);
        fs::write(
            root.join("index.html"),
            format!("<html><body><img src=\"/images/office-chair.webp\" alt=\"{alt}\" width=\"640\" height=\"480\"></body></html>"),
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());

        assert!(findings.iter().any(|finding| finding.rule_id == "IMG006"));
    }

    #[test]
    fn recommends_a_modern_source_for_legacy_raster_without_a_picture_fallback() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/legacy-photo.jpg"), b"legacy").unwrap();
        fs::write(root.join("images/responsive-photo.jpg"), b"fallback").unwrap();
        fs::write(root.join("images/responsive-photo.webp"), b"modern").unwrap();
        fs::write(
            root.join("index.html"),
            "<html><body><img src=\"/images/legacy-photo.jpg\" alt=\"A landscape\" width=\"640\" height=\"480\"><picture><source srcset=\"/images/responsive-photo.webp?v=1\"><img src=\"/images/responsive-photo.jpg\" alt=\"A city\" width=\"640\" height=\"480\"></picture></body></html>",
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());

        let modern_format_findings = findings
            .iter()
            .filter(|finding| finding.rule_id == "IMG005")
            .collect::<Vec<_>>();
        assert_eq!(modern_format_findings.len(), 1);
        assert!(
            modern_format_findings[0]
                .message
                .contains("legacy-photo.jpg")
        );
    }

    #[test]
    fn recommends_modern_format_when_the_local_picture_source_is_missing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/fallback.jpg"), b"fallback").unwrap();
        fs::write(
            root.join("index.html"),
            "<html><body><picture><source type=\"image/webp\" srcset=\"/images/missing.webp 1x\"><img src=\"/images/fallback.jpg\" alt=\"Office chair\" width=\"640\" height=\"480\"></picture></body></html>",
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());

        assert!(findings.iter().any(|finding| {
            finding.rule_id == "IMG004" && finding.message.contains("missing.webp")
        }));
        assert!(findings.iter().any(|finding| {
            finding.rule_id == "IMG005" && finding.message.contains("fallback.jpg")
        }));
    }

    #[test]
    fn audits_every_picture_source_candidate_for_missing_and_oversized_assets() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/fallback.jpg"), b"fallback").unwrap();
        fs::write(root.join("images/oversized.avif"), vec![0_u8; 100_001]).unwrap();
        fs::write(
            root.join("index.html"),
            "<html><body><picture><source type=\"image/avif\" srcset=\"/images/missing.avif 1x, /images/oversized.avif 2x\"><img src=\"/images/fallback.jpg\" alt=\"Office chair\" width=\"640\" height=\"480\"></picture></body></html>",
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());

        assert!(findings.iter().any(|finding| {
            finding.rule_id == "IMG004" && finding.message.contains("missing.avif")
        }));
        assert!(findings.iter().any(|finding| {
            finding.rule_id == "IMG003" && finding.message.contains("oversized.avif")
        }));
    }

    #[test]
    fn audits_filename_quality_for_every_responsive_image_candidate() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/hero-photo.jpg"), b"fallback").unwrap();
        fs::write(root.join("images/IMG_1234.jpg"), b"responsive").unwrap();
        fs::write(
            root.join("index.html"),
            "<html><body><img src=\"/images/hero-photo.jpg\" srcset=\"/images/IMG_1234.jpg 2x\" alt=\"A landscape\" width=\"640\" height=\"480\"></body></html>",
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());

        assert!(findings.iter().any(|finding| {
            finding.rule_id == "IMG001" && finding.message.contains("IMG_1234.jpg")
        }));
        assert!(findings.iter().any(|finding| {
            finding.rule_id == "IMG002" && finding.message.contains("IMG_1234.jpg")
        }));
    }

    #[test]
    fn audits_every_img_srcset_candidate_when_src_is_omitted() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(
            root.join("images/office-chair-large.jpg"),
            vec![0_u8; 100_001],
        )
        .unwrap();
        let alt = "a".repeat(126);
        fs::write(
            root.join("index.html"),
            format!("<html><body><img srcset=\"/images/office-chair.jpg 1x, /images/office-chair-large.jpg 2x\" alt=\"{alt}\" width=\"640\"></body></html>"),
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());

        assert!(findings.iter().any(|finding| {
            finding.rule_id == "IMG004" && finding.message.contains("office-chair.jpg")
        }));
        assert!(findings.iter().any(|finding| {
            finding.rule_id == "IMG003" && finding.message.contains("office-chair-large.jpg")
        }));
        assert!(findings.iter().any(|finding| finding.rule_id == "IMG006"));
        assert!(findings.iter().any(|finding| finding.rule_id == "IMG007"));
    }

    #[test]
    fn warns_when_a_local_raster_image_exceeds_the_default_100kb_budget() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/office-chair.jpg"), vec![0_u8; 100_001]).unwrap();
        fs::write(
            root.join("index.html"),
            "<html><body><img src=\"/images/office-chair.jpg\" alt=\"Office chair\" width=\"640\" height=\"480\"></body></html>",
        )
        .unwrap();
        let site = load_site(root).unwrap();

        let findings = run_image_rules(&site, &Config::default());

        assert!(findings.iter().any(|finding| finding.rule_id == "IMG003"));
    }

    #[test]
    fn detects_camera_generated_names_but_not_descriptive_names() {
        assert!(is_generic_filename("IMG_9482"));
        assert!(is_generic_filename("image-1"));
        assert!(!is_generic_filename("ergonomic-office-chair"));
    }

    #[test]
    fn allows_lowercase_unicode_kebab_filenames() {
        assert!(is_lowercase_kebab_filename("mañana-nube", "jpg"));
        assert!(!is_lowercase_kebab_filename("Mañana", "jpg"));
    }
}
