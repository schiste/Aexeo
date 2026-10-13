use anyhow::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::{decode_url_path, route_from_urlish};
use url::Url;

struct XmlTag<'a> {
    name: &'a str,
    local_name: &'a str,
    closing: bool,
    self_closing: bool,
    content_start: usize,
}

struct XmlTagIterator<'a> {
    text: &'a str,
    offset: usize,
}

impl<'a> XmlTagIterator<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, offset: 0 }
    }
}

impl<'a> Iterator for XmlTagIterator<'a> {
    type Item = XmlTag<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let start_rel = self.text.get(self.offset..)?.find('<')?;
            let tag_start = self.offset + start_rel;
            let remainder = &self.text[tag_start..];
            if remainder.starts_with("<!--") {
                let end_rel = remainder.find("-->")?;
                self.offset = tag_start + end_rel + "-->".len();
                continue;
            }
            if remainder.starts_with("<![CDATA[") {
                let end_rel = remainder.find("]]>")?;
                self.offset = tag_start + end_rel + 3;
                continue;
            }
            if remainder.starts_with("<?") {
                let end_rel = remainder.find("?>")?;
                self.offset = tag_start + end_rel + "?>".len();
                continue;
            }

            let tag_end = xml_tag_end(self.text, tag_start + 1)?;
            self.offset = tag_end + 1;
            let tag = self.text[tag_start + 1..tag_end].trim();
            if tag.is_empty() || tag.starts_with('!') {
                continue;
            }
            let closing = tag.starts_with('/');
            let name_start = usize::from(closing);
            let name = tag[name_start..]
                .trim_start()
                .split(|character: char| character.is_whitespace() || character == '/')
                .next()
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            return Some(XmlTag {
                name,
                local_name: name.rsplit(':').next().unwrap_or(name),
                closing,
                self_closing: !closing && tag.ends_with('/'),
                content_start: tag_end + 1,
            });
        }
    }
}

fn has_xml_element(text: &str, local_name: &str) -> bool {
    XmlTagIterator::new(text).any(|tag| !tag.closing && tag.local_name == local_name)
}

pub(super) fn read_sitemap_image_urls(
    path: &Path,
    root: &Path,
    allowed_origin: Option<&str>,
) -> Result<BTreeSet<String>> {
    let Ok(root) = fs::canonicalize(root) else {
        return Ok(BTreeSet::new());
    };
    let mut seen = BTreeSet::new();
    read_sitemap_image_urls_recursive(path, &root, &mut seen, allowed_origin)
}

pub(super) fn read_sitemap_image_urls_from_text(text: &str) -> Option<BTreeSet<String>> {
    if has_xml_element(text, "sitemapindex") || !has_xml_element(text, "urlset") {
        return None;
    }
    Some(read_image_loc_values(text).into_iter().collect())
}

fn read_sitemap_image_urls_recursive(
    path: &Path,
    root: &Path,
    seen: &mut BTreeSet<PathBuf>,
    allowed_origin: Option<&str>,
) -> Result<BTreeSet<String>> {
    let Some(path) = canonical_sitemap_path(path, root)? else {
        return Ok(BTreeSet::new());
    };
    if !seen.insert(path.clone()) {
        return Ok(BTreeSet::new());
    }
    let text = fs::read_to_string(&path)?;
    if has_xml_element(&text, "sitemapindex") {
        let mut image_urls = BTreeSet::new();
        for value in read_loc_values(&text) {
            let Some(relative_path) =
                sitemap_image_relative_path(&value, &path, root, allowed_origin)?
            else {
                continue;
            };
            image_urls.extend(read_sitemap_image_urls_recursive(
                &root.join(relative_path),
                root,
                seen,
                allowed_origin,
            )?);
        }
        return Ok(image_urls);
    }
    Ok(read_image_loc_values(&text).into_iter().collect())
}

fn read_image_loc_values(text: &str) -> Vec<String> {
    read_xml_loc_values(text, true)
}

fn read_xml_loc_values(text: &str, inside_image: bool) -> Vec<String> {
    let mut values = Vec::new();
    let mut image_depth = 0usize;
    for tag in XmlTagIterator::new(text) {
        if tag.closing {
            if tag.local_name == "image" {
                image_depth = image_depth.saturating_sub(1);
            }
            continue;
        }
        if tag.local_name == "image" {
            if !tag.self_closing {
                image_depth += 1;
            }
            continue;
        }
        if tag.local_name != "loc" || tag.self_closing || (image_depth > 0) != inside_image {
            continue;
        }
        if let Some((close_start, _)) = find_closing_xml_tag(text, tag.content_start, tag.name) {
            values.push(text[tag.content_start..close_start].trim().to_string());
        }
    }
    values
}

fn xml_tag_end(text: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, character) in text[start..].char_indices() {
        if let Some(quote_character) = quote {
            if character == quote_character {
                quote = None;
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '>' {
            return Some(start + offset);
        }
    }
    None
}

fn find_closing_xml_tag(text: &str, start: usize, name: &str) -> Option<(usize, usize)> {
    let needle = format!("</{name}");
    let mut offset = start;
    while let Some(relative_start) = text[offset..].find(&needle) {
        let close_start = offset + relative_start;
        let after_name = close_start + needle.len();
        let boundary = text[after_name..].chars().next()?;
        if boundary.is_whitespace() || boundary == '>' {
            let close_end = xml_tag_end(text, after_name)? + 1;
            return Some((close_start, close_end));
        }
        offset = after_name;
    }
    None
}

fn canonical_sitemap_path(path: &Path, root: &Path) -> Result<Option<PathBuf>> {
    let canonical_path = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(canonical_path.starts_with(root).then_some(canonical_path))
}

fn sitemap_relative_path(value: &str, current: &Path, root: &Path) -> Option<PathBuf> {
    let value = value.trim();
    let (raw_path, root_relative) = if value.starts_with("//") {
        let url = Url::parse(&format!("https:{value}")).ok()?;
        (url.path().to_string(), true)
    } else if let Ok(url) = Url::parse(value) {
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        (url.path().to_string(), true)
    } else {
        let path = value.split(['?', '#']).next()?.trim();
        (path.to_string(), path.starts_with('/'))
    };

    let decoded_path = decode_url_path(&raw_path)?;
    let path = decoded_path.trim_start_matches('/');
    let base = if root_relative {
        PathBuf::new()
    } else {
        current.strip_prefix(root).ok()?.parent()?.to_path_buf()
    };
    let candidate = base.join(path);
    let mut relative = PathBuf::new();
    for component in candidate.components() {
        match component {
            std::path::Component::Normal(value) => relative.push(value),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !relative.pop() {
                    return None;
                }
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

fn sitemap_image_relative_path(
    value: &str,
    current: &Path,
    root: &Path,
    allowed_origin: Option<&str>,
) -> Result<Option<PathBuf>> {
    let value = value.trim();
    let parsed_url = if value.starts_with("//") {
        let scheme = allowed_origin
            .and_then(|origin| Url::parse(origin).ok())
            .map(|url| url.scheme().to_string())
            .unwrap_or_else(|| "https".to_string());
        Url::parse(&format!("{scheme}:{value}")).ok()
    } else {
        Url::parse(value).ok()
    };
    if let Some(url) = parsed_url.filter(|url| matches!(url.scheme(), "http" | "https")) {
        let Some(allowed_origin) = allowed_origin else {
            anyhow::bail!(
                "cannot safely resolve nested image sitemap URL without a configured or canonical site origin"
            );
        };
        let expected_origin = Url::parse(allowed_origin)?.origin().ascii_serialization();
        let actual_origin = url.origin().ascii_serialization();
        if !actual_origin.eq_ignore_ascii_case(&expected_origin) {
            anyhow::bail!(
                "nested image sitemap URL has origin {actual_origin}, which differs from the site's {expected_origin}"
            );
        }
    }
    Ok(sitemap_relative_path(value, current, root))
}

fn sitemap_route_relative_path(
    value: &str,
    current: &Path,
    root: &Path,
    allowed_origin: Option<&str>,
) -> Result<Option<PathBuf>> {
    let value = value.trim();
    let parsed_url = if value.starts_with("//") {
        let Some(allowed_origin) = allowed_origin else {
            anyhow::bail!(
                "cannot safely resolve nested sitemap URL without a canonical site origin"
            );
        };
        let scheme = Url::parse(allowed_origin)?.scheme().to_string();
        Some(Url::parse(&format!("{scheme}:{value}"))?)
    } else {
        Url::parse(value).ok()
    };
    if let Some(url) = parsed_url.filter(|url| matches!(url.scheme(), "http" | "https")) {
        let Some(allowed_origin) = allowed_origin else {
            anyhow::bail!(
                "cannot safely resolve nested sitemap URL without a canonical site origin"
            );
        };
        let expected_origin = Url::parse(allowed_origin)?.origin().ascii_serialization();
        let actual_origin = url.origin().ascii_serialization();
        if !actual_origin.eq_ignore_ascii_case(&expected_origin) {
            anyhow::bail!(
                "nested sitemap URL has origin {actual_origin}, which differs from the site's {expected_origin}"
            );
        }
    }
    Ok(sitemap_relative_path(value, current, root))
}

pub(super) fn read_sitemap_routes(
    path: &Path,
    root: &Path,
    allowed_origin: Option<&str>,
) -> (BTreeSet<String>, Option<String>) {
    if !path.exists() {
        return (BTreeSet::new(), None);
    }
    let canonical_root = match fs::canonicalize(root) {
        Ok(root) => root,
        Err(error) => return (BTreeSet::new(), Some(error.to_string())),
    };
    let mut seen = BTreeSet::new();
    match read_sitemap_routes_recursive(path, &canonical_root, &mut seen, allowed_origin) {
        Ok(routes) => (routes, None),
        Err(error) => (BTreeSet::new(), Some(error.to_string())),
    }
}

pub(super) fn read_sitemap_routes_from_text(text: &str) -> Result<BTreeSet<String>> {
    if has_xml_element(text, "sitemapindex") {
        return Ok(BTreeSet::new());
    }
    if !has_xml_element(text, "urlset") {
        anyhow::bail!("invalid sitemap XML")
    }
    Ok(read_loc_values(text)
        .into_iter()
        .filter_map(|value| route_from_urlish(&value))
        .collect())
}

fn read_loc_values(text: &str) -> Vec<String> {
    read_xml_loc_values(text, false)
}

fn read_sitemap_routes_recursive(
    path: &Path,
    root: &Path,
    seen: &mut BTreeSet<PathBuf>,
    allowed_origin: Option<&str>,
) -> Result<BTreeSet<String>> {
    let Some(path) = canonical_sitemap_path(path, root)? else {
        return Ok(BTreeSet::new());
    };
    if !seen.insert(path.clone()) {
        return Ok(BTreeSet::new());
    }
    let text = fs::read_to_string(&path)?;
    if has_xml_element(&text, "sitemapindex") {
        let mut routes = BTreeSet::new();
        for value in read_loc_values(&text) {
            if let Some(relative_path) =
                sitemap_route_relative_path(&value, &path, root, allowed_origin)?
            {
                routes.extend(read_sitemap_routes_recursive(
                    &root.join(relative_path),
                    root,
                    seen,
                    allowed_origin,
                )?);
            }
        }
        return Ok(routes);
    }
    read_sitemap_routes_from_text(&text)
}
