use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub(crate) struct FetchResult {
    pub(crate) status_code: Option<u16>,
    pub(crate) content_type: Option<String>,
    pub(crate) body: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) effective_url: String,
}

pub(crate) fn unique_runtime_dir() -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("seogeo-runtime-{}-{}", std::process::id(), nonce));
    fs::create_dir_all(&path)?;
    Ok(path)
}

pub(crate) fn normalize_base_url(base_url: &str) -> String {
    format!("{}/", base_url.trim_end_matches('/'))
}

pub(crate) fn host_for_url(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    after_scheme
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn parse_headers(raw: &str) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("HTTP/") {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    headers
}

pub(crate) fn is_html_content_type(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(media_type.as_str(), "text/html" | "application/xhtml+xml")
}

pub(crate) fn fetch_with_curl(
    url: &str,
    headers: &BTreeMap<String, String>,
    basic_auth: &BTreeMap<String, String>,
) -> Result<FetchResult> {
    let body_path = unique_runtime_dir()?.join("body.txt");
    let headers_path = unique_runtime_dir()?.join("headers.txt");
    let mut command = ProcessCommand::new("curl");
    command.arg("-sS").arg("-L");
    command.arg("-D").arg(&headers_path);
    command.arg("-o").arg(&body_path);
    command
        .arg("-w")
        .arg("%{http_code}\n%{content_type}\n%{url_effective}");
    for (key, value) in headers {
        command.arg("-H").arg(format!("{}: {}", key, value));
    }
    if let (Some(username), Some(password)) =
        (basic_auth.get("username"), basic_auth.get("password"))
    {
        command.arg("-u").arg(format!("{}:{}", username, password));
    }
    command.arg(url);
    let output = command
        .output()
        .with_context(|| format!("failed to run curl for {}", url))?;
    let metadata = String::from_utf8_lossy(&output.stdout);
    let mut lines = metadata.lines();
    let status_code = lines.next().and_then(|value| value.parse::<u16>().ok());
    let content_type = lines
        .next()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let effective_url = lines.next().unwrap_or(url).trim().to_string();
    let body = fs::read_to_string(&body_path).ok();
    let headers_map = fs::read_to_string(&headers_path)
        .map(|raw| parse_headers(&raw))
        .unwrap_or_default();
    let _ = fs::remove_file(body_path);
    let _ = fs::remove_file(headers_path);
    if !output.status.success() && body.is_none() {
        return Ok(FetchResult {
            status_code,
            content_type: None,
            body: None,
            headers: BTreeMap::new(),
            effective_url,
        });
    }
    Ok(FetchResult {
        status_code,
        content_type,
        body,
        headers: headers_map,
        effective_url,
    })
}

pub(crate) fn write_optional_artifact(
    snapshot_root: &Path,
    base_url: &str,
    name: &str,
    headers: &BTreeMap<String, String>,
    basic_auth: &BTreeMap<String, String>,
) -> Result<()> {
    let artifact_url = format!("{}{}", base_url, name);
    let fetched = fetch_with_curl(&artifact_url, headers, basic_auth)?;
    if let Some(body) = fetched.body
        && fetched.status_code.unwrap_or(200) < 400
    {
        fs::write(snapshot_root.join(name), body)?;
    }
    Ok(())
}
