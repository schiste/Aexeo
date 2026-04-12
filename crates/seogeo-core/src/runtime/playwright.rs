use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::http::FetchResult;
use crate::config::RuntimeConfig;

const PLAYWRIGHT_INLINE_RUNNER: &str = r#"
import fs from 'node:fs';
import path from 'node:path';
import { chromium } from 'playwright';

const spec = JSON.parse(Buffer.from(process.argv[2], 'base64').toString('utf8'));

function sanitizeSegment(value) {
  return (value || 'index').replace(/[^a-zA-Z0-9._-]+/g, '-').replace(/^-+|-+$/g, '') || 'index';
}

function artifactStem(urlString) {
  try {
    const url = new URL(urlString);
    const pathname = url.pathname === '/' ? 'index' : url.pathname.replace(/^\/+/, '').replace(/\/+$/, '') || 'index';
    return sanitizeSegment(pathname);
  } catch {
    return 'page';
  }
}

function normalizeCookies(cookies, urlString) {
  return (cookies || [])
    .map((cookie) => {
      if (!cookie || typeof cookie !== 'object') return null;
      if (typeof cookie.name !== 'string' || typeof cookie.value !== 'string') return null;
      return {
        name: cookie.name,
        value: cookie.value,
        url: typeof cookie.url === 'string' ? cookie.url : urlString,
        domain: typeof cookie.domain === 'string' ? cookie.domain : undefined,
        path: typeof cookie.path === 'string' ? cookie.path : undefined,
        httpOnly: Boolean(cookie.httpOnly),
        secure: Boolean(cookie.secure),
        expires: typeof cookie.expires === 'number' ? cookie.expires : undefined,
        sameSite: typeof cookie.sameSite === 'string' ? cookie.sameSite : undefined
      };
    })
    .filter(Boolean);
}

const browser = await chromium.launch({ headless: true });
const contextOptions = {
  extraHTTPHeaders: spec.headers || {},
};
if (spec.basicAuth && spec.basicAuth.username && spec.basicAuth.password) {
  contextOptions.httpCredentials = {
    username: spec.basicAuth.username,
    password: spec.basicAuth.password
  };
}
const context = await browser.newContext(contextOptions);
const cookies = normalizeCookies(spec.cookies, spec.url);
if (cookies.length > 0) {
  await context.addCookies(cookies);
}

const page = await context.newPage();
const consoleEvents = [];
const networkEvents = [];

if (spec.captureConsole) {
  page.on('console', (message) => {
    consoleEvents.push({ type: message.type(), text: message.text() });
  });
}
if (spec.captureNetwork) {
  page.on('response', async (response) => {
    networkEvents.push({
      url: response.url(),
      status: response.status(),
      resourceType: response.request().resourceType()
    });
  });
}
if (spec.captureTrace) {
  await context.tracing.start({ screenshots: true, snapshots: true });
}

let mainResponse = null;
try {
  mainResponse = await page.goto(spec.url, { waitUntil: spec.waitUntil || 'networkidle' });
  const html = await page.content();
  const effectiveUrl = page.url();
  const headers = mainResponse ? await mainResponse.allHeaders() : {};
  const contentTypeHeader = Object.entries(headers).find(([key]) => key.toLowerCase() === 'content-type');
  const artifactDir = spec.artifactDir ? path.resolve(spec.artifactDir) : null;
  let screenshotPath = null;
  let tracePath = null;
  let consolePath = null;
  let networkPath = null;

  if (artifactDir) {
    fs.mkdirSync(artifactDir, { recursive: true });
    const stem = artifactStem(effectiveUrl);
    if (spec.captureScreenshot) {
      screenshotPath = path.join(artifactDir, `${stem}.png`);
      await page.screenshot({ path: screenshotPath, fullPage: true });
    }
    if (spec.captureConsole && consoleEvents.length > 0) {
      consolePath = path.join(artifactDir, `${stem}.console.json`);
      fs.writeFileSync(consolePath, JSON.stringify(consoleEvents, null, 2));
    }
    if (spec.captureNetwork && networkEvents.length > 0) {
      networkPath = path.join(artifactDir, `${stem}.network.json`);
      fs.writeFileSync(networkPath, JSON.stringify(networkEvents, null, 2));
    }
    if (spec.captureTrace) {
      tracePath = path.join(artifactDir, `${stem}.trace.zip`);
      await context.tracing.stop({ path: tracePath });
    }
  } else if (spec.captureTrace) {
    await context.tracing.stop();
  }

  const payload = {
    statusCode: mainResponse ? mainResponse.status() : null,
    contentType: contentTypeHeader ? contentTypeHeader[1] : 'text/html; charset=utf-8',
    body: html,
    headers,
    effectiveUrl,
    artifacts: {
      screenshotPath,
      tracePath,
      consolePath,
      networkPath
    }
  };
  process.stdout.write(JSON.stringify(payload));
} finally {
  await context.close();
  await browser.close();
}
"#;

#[derive(Debug, Clone)]
struct PlaywrightSpec<'a> {
    url: &'a str,
    headers: &'a BTreeMap<String, String>,
    cookies: &'a [BTreeMap<String, String>],
    basic_auth: &'a BTreeMap<String, String>,
    wait_until: &'a str,
    capture_trace: bool,
    capture_screenshot: bool,
    capture_console: bool,
    capture_network: bool,
    artifact_dir: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaywrightDoctor {
    pub available: bool,
    pub mode: String,
    pub executable: String,
    pub message: String,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf()
}

fn configured_playwright_executable() -> Option<PathBuf> {
    std::env::var_os("SEOGEO_PLAYWRIGHT_EXECUTABLE").map(PathBuf::from)
}

fn encode_spec(spec: &PlaywrightSpec<'_>) -> Result<String> {
    Ok(BASE64.encode(serde_json::to_vec(&serde_json::json!({
        "url": spec.url,
        "headers": spec.headers,
        "cookies": spec.cookies,
        "basicAuth": spec.basic_auth,
        "waitUntil": spec.wait_until,
        "captureTrace": spec.capture_trace,
        "captureScreenshot": spec.capture_screenshot,
        "captureConsole": spec.capture_console,
        "captureNetwork": spec.capture_network,
        "artifactDir": spec.artifact_dir
    }))?))
}

pub(crate) fn playwright_is_available() -> bool {
    probe_playwright_runtime().available
}

pub(crate) fn probe_playwright_runtime() -> PlaywrightDoctor {
    probe_playwright_runtime_with(configured_playwright_executable())
}

fn probe_playwright_runtime_with(executable: Option<PathBuf>) -> PlaywrightDoctor {
    if let Some(executable) = executable {
        return PlaywrightDoctor {
            available: executable.exists(),
            mode: "custom_runner".to_string(),
            executable: executable.display().to_string(),
            message: if executable.exists() {
                "custom Playwright runner path exists".to_string()
            } else {
                "custom Playwright runner path does not exist".to_string()
            },
        };
    }
    let output = Command::new("node")
        .arg("--input-type=module")
        .arg("-e")
        .arg("import('playwright').then(async ({ chromium }) => { const browser = await chromium.launch({ headless: true }); await browser.close(); process.stdout.write('ok'); }).catch((error) => { console.error(String(error)); process.exit(1); })")
        .current_dir(repo_root())
        .output();
    match output {
        Ok(result) if result.status.success() => PlaywrightDoctor {
            available: true,
            mode: "node_module".to_string(),
            executable: "node".to_string(),
            message: "Playwright module imported and Chromium launched successfully".to_string(),
        },
        Ok(result) => PlaywrightDoctor {
            available: false,
            mode: "node_module".to_string(),
            executable: "node".to_string(),
            message: String::from_utf8_lossy(&result.stderr).trim().to_string(),
        },
        Err(error) => PlaywrightDoctor {
            available: false,
            mode: "node_module".to_string(),
            executable: "node".to_string(),
            message: error.to_string(),
        },
    }
}

pub(crate) fn fetch_with_playwright(url: &str, runtime: &RuntimeConfig<'_>) -> Result<FetchResult> {
    fetch_with_playwright_using(configured_playwright_executable(), url, runtime)
}

fn fetch_with_playwright_using(
    executable: Option<PathBuf>,
    url: &str,
    runtime: &RuntimeConfig<'_>,
) -> Result<FetchResult> {
    let spec = PlaywrightSpec {
        url,
        headers: runtime.crawl_headers,
        cookies: runtime.crawl_cookies,
        basic_auth: runtime.crawl_basic_auth,
        wait_until: runtime.browser_wait_until,
        capture_trace: runtime.crawl_capture_trace,
        capture_screenshot: runtime.crawl_capture_screenshot,
        capture_console: runtime.crawl_capture_console,
        capture_network: runtime.crawl_capture_network,
        artifact_dir: runtime.crawl_artifact_dir,
    };
    let encoded_spec = encode_spec(&spec)?;
    let mut command = if let Some(executable) = executable.as_ref() {
        let mut command = Command::new(executable);
        command.arg(&encoded_spec);
        command
    } else {
        let mut command = Command::new("node");
        command
            .arg("--input-type=module")
            .arg("-")
            .arg(&encoded_spec)
            .current_dir(repo_root())
            .stdin(Stdio::piped());
        command
    };
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn playwright runtime for {url}"))?;
    if executable.is_none()
        && let Some(stdin) = child.stdin.as_mut()
    {
        stdin.write_all(PLAYWRIGHT_INLINE_RUNNER.as_bytes())?;
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("failed to execute playwright runtime for {url}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("playwright runtime failed for {url}: {stderr}");
    }
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("invalid playwright runtime response for {url}"))?;
    let headers = payload["headers"]
        .as_object()
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(key, value)| {
                    Some((key.to_ascii_lowercase(), value.as_str()?.to_string()))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    Ok(FetchResult {
        status_code: payload["statusCode"].as_u64().map(|value| value as u16),
        content_type: payload["contentType"].as_str().map(str::to_string),
        body: payload["body"].as_str().map(str::to_string),
        headers,
        effective_url: payload["effectiveUrl"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| url.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::{fetch_with_playwright_using, probe_playwright_runtime_with};
    use crate::config::Config;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn playwright_availability_honors_explicit_runner() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runner = temp_dir.path().join("runner.sh");
        fs::write(&runner, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&runner).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&runner, permissions).unwrap();
        assert!(probe_playwright_runtime_with(Some(runner)).available);
    }

    #[test]
    fn playwright_fetch_uses_custom_runner_contract() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runner = temp_dir.path().join("runner.sh");
        fs::write(
            &runner,
            "#!/bin/sh\nprintf '%s' '{\"statusCode\":200,\"contentType\":\"text/html\",\"body\":\"<html><body><a href=\\\"/about\\\">About</a></body></html>\",\"headers\":{\"content-type\":\"text/html\"},\"effectiveUrl\":\"https://example.com/\"}'\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&runner).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&runner, permissions).unwrap();
        let config = Config::default();
        let fetched =
            fetch_with_playwright_using(Some(runner), "https://example.com", &config.runtime())
                .unwrap();
        assert_eq!(fetched.status_code, Some(200));
        assert_eq!(fetched.effective_url, "https://example.com/");
        assert!(fetched.body.unwrap().contains("/about"));
    }
}
