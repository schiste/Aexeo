use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::http::FetchResult;
use crate::config::RuntimeConfig;

const PLAYWRIGHT_INLINE_RUNNER: &str = r#"
import fs from 'node:fs';
import path from 'node:path';
import { chromium } from 'playwright';

const spec = JSON.parse(Buffer.from(process.argv.at(-1), 'base64').toString('utf8'));

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

const PLAYWRIGHT_SESSION_RUNNER: &str = r#"
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import { chromium } from 'playwright';

const baseSpec = JSON.parse(Buffer.from(process.argv.at(-1), 'base64').toString('utf8'));

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
  extraHTTPHeaders: baseSpec.headers || {},
};
if (baseSpec.basicAuth && baseSpec.basicAuth.username && baseSpec.basicAuth.password) {
  contextOptions.httpCredentials = {
    username: baseSpec.basicAuth.username,
    password: baseSpec.basicAuth.password
  };
}
const context = await browser.newContext(contextOptions);
const initialCookies = normalizeCookies(baseSpec.cookies, baseSpec.initialUrl || 'https://example.com');
if (initialCookies.length > 0) {
  await context.addCookies(initialCookies);
}
const page = await context.newPage();

async function fetchUrl(url) {
  const consoleEvents = [];
  const networkEvents = [];
  const consoleListener = (message) => {
    consoleEvents.push({ type: message.type(), text: message.text() });
  };
  const networkListener = async (response) => {
    networkEvents.push({
      url: response.url(),
      status: response.status(),
      resourceType: response.request().resourceType()
    });
  };

  if (baseSpec.captureConsole) {
    page.on('console', consoleListener);
  }
  if (baseSpec.captureNetwork) {
    page.on('response', networkListener);
  }
  if (baseSpec.captureTrace) {
    await context.tracing.start({ screenshots: true, snapshots: true });
  }

  try {
    const mainResponse = await page.goto(url, { waitUntil: baseSpec.waitUntil || 'networkidle' });
    const html = await page.content();
    const effectiveUrl = page.url();
    const headers = mainResponse ? await mainResponse.allHeaders() : {};
    const contentTypeHeader = Object.entries(headers).find(([key]) => key.toLowerCase() === 'content-type');
    const artifactDir = baseSpec.artifactDir ? path.resolve(baseSpec.artifactDir) : null;
    let screenshotPath = null;
    let tracePath = null;
    let consolePath = null;
    let networkPath = null;

    if (artifactDir) {
      fs.mkdirSync(artifactDir, { recursive: true });
      const stem = artifactStem(effectiveUrl);
      if (baseSpec.captureScreenshot) {
        screenshotPath = path.join(artifactDir, `${stem}.png`);
        await page.screenshot({ path: screenshotPath, fullPage: true });
      }
      if (baseSpec.captureConsole && consoleEvents.length > 0) {
        consolePath = path.join(artifactDir, `${stem}.console.json`);
        fs.writeFileSync(consolePath, JSON.stringify(consoleEvents, null, 2));
      }
      if (baseSpec.captureNetwork && networkEvents.length > 0) {
        networkPath = path.join(artifactDir, `${stem}.network.json`);
        fs.writeFileSync(networkPath, JSON.stringify(networkEvents, null, 2));
      }
      if (baseSpec.captureTrace) {
        tracePath = path.join(artifactDir, `${stem}.trace.zip`);
        await context.tracing.stop({ path: tracePath });
      }
    } else if (baseSpec.captureTrace) {
      await context.tracing.stop();
    }

    return {
      ok: true,
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
  } catch (error) {
    if (baseSpec.captureTrace) {
      try {
        await context.tracing.stop();
      } catch {}
    }
    return {
      ok: false,
      error: String(error)
    };
  } finally {
    if (baseSpec.captureConsole) {
      page.off('console', consoleListener);
    }
    if (baseSpec.captureNetwork) {
      page.off('response', networkListener);
    }
  }
}

const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
try {
  for await (const line of rl) {
    if (!line) continue;
    if (line === '__AEXEO_EXIT__') break;
    let payload;
    try {
      const request = JSON.parse(Buffer.from(line, 'base64').toString('utf8'));
      payload = await fetchUrl(request.url);
    } catch (error) {
      payload = { ok: false, error: String(error) };
    }
    process.stdout.write(JSON.stringify(payload) + '\n');
  }
} finally {
  rl.close();
  await context.close();
  await browser.close();
}
"#;

const PLAYWRIGHT_PROBE_RUNNER: &str = r#"
import { chromium } from 'playwright';

try {
  const browser = await chromium.launch({ headless: true });
  await browser.close();
  process.stdout.write('ok');
} catch (error) {
  console.error(String(error));
  process.exit(1);
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

#[derive(Debug)]
pub(crate) struct PlaywrightSession {
    config: PlaywrightOwnedConfig,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
}

#[derive(Debug)]
pub(crate) enum PlaywrightFetcher {
    Session(PlaywrightSession),
    OneShot(PathBuf, PlaywrightOwnedConfig),
}

#[derive(Debug, Clone)]
pub(crate) struct PlaywrightOwnedConfig {
    headers: BTreeMap<String, String>,
    cookies: Vec<BTreeMap<String, String>>,
    basic_auth: BTreeMap<String, String>,
    wait_until: String,
    capture_trace: bool,
    capture_screenshot: bool,
    capture_console: bool,
    capture_network: bool,
    artifact_dir: String,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf()
}

fn materialize_playwright_runner(file_name: &str, source: &str) -> Result<PathBuf> {
    let runner_path = repo_root().join("target").join(file_name);
    if let Some(parent) = runner_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create playwright runner dir {}",
                parent.display()
            )
        })?;
    }
    fs::write(&runner_path, source).with_context(|| {
        format!(
            "failed to write playwright runner {}",
            runner_path.display()
        )
    })?;
    Ok(runner_path)
}

fn configured_playwright_executable() -> Option<PathBuf> {
    std::env::var_os("AEXEO_PLAYWRIGHT_EXECUTABLE").map(PathBuf::from)
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

impl PlaywrightFetcher {
    pub(crate) fn new(runtime: &RuntimeConfig<'_>) -> Result<Self> {
        if let Some(executable) = configured_playwright_executable() {
            return Ok(Self::OneShot(
                executable,
                PlaywrightOwnedConfig::from_runtime(runtime),
            ));
        }
        Ok(Self::Session(PlaywrightSession::spawn(runtime)?))
    }

    pub(crate) fn fetch(&mut self, url: &str) -> Result<FetchResult> {
        match self {
            Self::Session(session) => session.fetch(url),
            Self::OneShot(executable, config) => {
                fetch_with_playwright_spec(Some(executable.clone()), &config.spec_for_url(url))
            }
        }
    }
}

impl PlaywrightOwnedConfig {
    fn from_runtime(runtime: &RuntimeConfig<'_>) -> Self {
        Self {
            headers: runtime.crawl_headers.clone(),
            cookies: runtime.crawl_cookies.to_vec(),
            basic_auth: runtime.crawl_basic_auth.clone(),
            wait_until: runtime.browser_wait_until.to_string(),
            capture_trace: runtime.crawl_capture_trace,
            capture_screenshot: runtime.crawl_capture_screenshot,
            capture_console: runtime.crawl_capture_console,
            capture_network: runtime.crawl_capture_network,
            artifact_dir: runtime.crawl_artifact_dir.to_string(),
        }
    }

    fn spec_for_url<'a>(&'a self, url: &'a str) -> PlaywrightSpec<'a> {
        PlaywrightSpec {
            url,
            headers: &self.headers,
            cookies: &self.cookies,
            basic_auth: &self.basic_auth,
            wait_until: &self.wait_until,
            capture_trace: self.capture_trace,
            capture_screenshot: self.capture_screenshot,
            capture_console: self.capture_console,
            capture_network: self.capture_network,
            artifact_dir: &self.artifact_dir,
        }
    }
}

impl PlaywrightSession {
    fn spawn(runtime: &RuntimeConfig<'_>) -> Result<Self> {
        Self::spawn_from_config(PlaywrightOwnedConfig::from_runtime(runtime))
    }

    fn spawn_from_config(config: PlaywrightOwnedConfig) -> Result<Self> {
        let encoded_spec = encode_spec(&config.spec_for_url("https://example.com"))?;
        let runner_path = materialize_playwright_runner(
            "aexeo-playwright-session-runner.mjs",
            PLAYWRIGHT_SESSION_RUNNER,
        )?;
        let mut child = Command::new("node")
            .arg(runner_path)
            .arg(&encoded_spec)
            .current_dir(repo_root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn persistent playwright runtime")?;
        let stdin = child
            .stdin
            .take()
            .context("playwright session stdin unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("playwright session stdout unavailable")?;
        let stderr = child
            .stderr
            .take()
            .context("playwright session stderr unavailable")?;
        Ok(Self {
            config,
            child,
            stdin,
            stdout: BufReader::new(stdout),
            stderr,
        })
    }

    fn fetch(&mut self, url: &str) -> Result<FetchResult> {
        match self.fetch_once(url) {
            Ok(result) => Ok(result),
            Err(error) if is_transient_playwright_process_error(&error.to_string()) => {
                self.restart()?;
                self.fetch_once(url)
            }
            Err(error) => Err(error),
        }
    }

    fn fetch_once(&mut self, url: &str) -> Result<FetchResult> {
        let request = BASE64.encode(serde_json::to_vec(&serde_json::json!({ "url": url }))?);
        if let Err(error) = self.stdin.write_all(request.as_bytes()) {
            let stderr = self.read_stderr();
            bail!("failed to send playwright request for {url}: {error}; {stderr}");
        }
        if let Err(error) = self.stdin.write_all(b"\n") {
            let stderr = self.read_stderr();
            bail!("failed to terminate playwright request for {url}: {error}; {stderr}");
        }
        if let Err(error) = self.stdin.flush() {
            let stderr = self.read_stderr();
            bail!("failed to flush playwright request for {url}: {error}; {stderr}");
        }

        let mut line = String::new();
        let read = self.stdout.read_line(&mut line)?;
        if read == 0 {
            let stderr = self.read_stderr();
            bail!("playwright runtime terminated while fetching {url}: {stderr}");
        }
        let payload: serde_json::Value = serde_json::from_str(line.trim_end())
            .with_context(|| format!("invalid persistent playwright response for {url}"))?;
        if !payload["ok"].as_bool().unwrap_or(false) {
            bail!(
                "playwright runtime failed for {url}: {}",
                payload["error"].as_str().unwrap_or("unknown error")
            );
        }
        Ok(fetch_result_from_payload(url, &payload))
    }

    fn restart(&mut self) -> Result<()> {
        let replacement = Self::spawn_from_config(self.config.clone())?;
        let old = std::mem::replace(self, replacement);
        drop(old);
        Ok(())
    }

    fn read_stderr(&mut self) -> String {
        let mut stderr = String::new();
        let _ = self.stderr.read_to_string(&mut stderr);
        stderr.trim().to_string()
    }
}

impl Drop for PlaywrightSession {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"__AEXEO_EXIT__\n");
        let _ = self.stdin.flush();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
    let mut output = run_playwright_probe_once();
    for _ in 0..2 {
        let should_retry = match &output {
            Ok(result) if !result.status.success() => {
                is_transient_playwright_process_error(&String::from_utf8_lossy(&result.stderr))
            }
            Err(error) => is_transient_playwright_process_error(&error.to_string()),
            _ => false,
        };
        if !should_retry {
            break;
        }
        thread::sleep(Duration::from_millis(50));
        output = run_playwright_probe_once();
    }
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

fn run_playwright_probe_once() -> std::io::Result<std::process::Output> {
    let runner_path =
        materialize_playwright_runner("aexeo-playwright-probe-runner.mjs", PLAYWRIGHT_PROBE_RUNNER)
            .map_err(std::io::Error::other)?;
    Command::new("node")
        .arg(runner_path)
        .current_dir(repo_root())
        .output()
}

fn is_transient_playwright_process_error(message: &str) -> bool {
    message.contains("EINTR")
        && (message.contains("process.cwd")
            || message.contains("uv_cwd")
            || message.contains("current working directory"))
}

#[cfg(test)]
fn fetch_with_playwright_using(
    executable: Option<PathBuf>,
    url: &str,
    runtime: &RuntimeConfig<'_>,
) -> Result<FetchResult> {
    fetch_with_playwright_spec(
        executable,
        &PlaywrightSpec {
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
        },
    )
}

fn fetch_with_playwright_spec(
    executable: Option<PathBuf>,
    spec: &PlaywrightSpec<'_>,
) -> Result<FetchResult> {
    let encoded_spec = encode_spec(spec)?;
    let mut command = if let Some(executable) = executable.as_ref() {
        let mut command = Command::new(executable);
        command.arg(&encoded_spec);
        command
    } else {
        let runner_path = materialize_playwright_runner(
            "aexeo-playwright-inline-runner.mjs",
            PLAYWRIGHT_INLINE_RUNNER,
        )?;
        let mut command = Command::new("node");
        command
            .arg(runner_path)
            .arg(&encoded_spec)
            .current_dir(repo_root());
        command
    };
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let child = command
        .spawn()
        .with_context(|| format!("failed to spawn playwright runtime for {}", spec.url))?;
    let output = child
        .wait_with_output()
        .with_context(|| format!("failed to execute playwright runtime for {}", spec.url))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("playwright runtime failed for {}: {stderr}", spec.url);
    }
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("invalid playwright runtime response for {}", spec.url))?;
    Ok(fetch_result_from_payload(spec.url, &payload))
}

fn fetch_result_from_payload(url: &str, payload: &serde_json::Value) -> FetchResult {
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
    FetchResult {
        status_code: payload["statusCode"].as_u64().map(|value| value as u16),
        content_type: payload["contentType"].as_str().map(str::to_string),
        body: payload["body"].as_str().map(str::to_string),
        headers,
        effective_url: payload["effectiveUrl"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| url.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PlaywrightOwnedConfig, encode_spec, fetch_with_playwright_using,
        is_transient_playwright_process_error, probe_playwright_runtime_with,
    };
    use crate::config::Config;
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

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

    #[test]
    fn playwright_detects_transient_cwd_eintr() {
        assert!(is_transient_playwright_process_error(
            "Error: EINTR: process.cwd failed with error interrupted system call, uv_cwd"
        ));
        assert!(!is_transient_playwright_process_error(
            "Error: playwright module missing"
        ));
    }

    /// Write an executable shell script that stands in for the Node
    /// playwright runner, so the process-orchestration and output-parsing
    /// half of this module is reachable without a browser installed.
    ///
    /// This is the same seam the two existing tests use, and it is the only
    /// reason the fetch path can be covered at all: 54% of this file is
    /// otherwise unreachable on a machine without Playwright.
    fn fake_runner(dir: &Path, script: &str) -> PathBuf {
        let runner = dir.join("runner.sh");
        fs::write(&runner, script).expect("write runner");
        let mut permissions = fs::metadata(&runner).expect("stat").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&runner, permissions).expect("chmod");
        runner
    }

    fn json_runner(body: &str) -> String {
        format!("#!/bin/sh\nprintf '%s' '{body}'\n")
    }

    /// The spec is handed to Node as base64-encoded JSON, and Node is the
    /// only consumer. If a field is dropped or renamed here, the browser
    /// silently receives a default instead of the configured value — headers,
    /// cookies, and basic auth included.
    #[test]
    fn encoded_spec_carries_every_configured_field() {
        use base64::Engine as _;
        let config = PlaywrightOwnedConfig {
            headers: BTreeMap::from([("x-test".to_string(), "yes".to_string())]),
            cookies: vec![BTreeMap::from([(
                "name".to_string(),
                "session".to_string(),
            )])],
            basic_auth: BTreeMap::from([
                ("username".to_string(), "user".to_string()),
                ("password".to_string(), "pass".to_string()),
            ]),
            wait_until: "networkidle".to_string(),
            capture_trace: true,
            capture_screenshot: false,
            capture_console: true,
            capture_network: false,
            artifact_dir: "/tmp/aexeo-artifacts".to_string(),
        };
        let encoded = encode_spec(&config.spec_for_url("https://example.com/")).expect("encode");
        let decoded = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
        )
        .expect("spec is base64 of utf-8 json");
        let value: serde_json::Value = serde_json::from_str(&decoded).expect("spec is json");

        assert_eq!(value["url"], "https://example.com/");
        assert_eq!(value["headers"]["x-test"], "yes");
        assert_eq!(value["cookies"][0]["name"], "session");
        assert_eq!(value["basicAuth"]["username"], "user");
        assert_eq!(value["basicAuth"]["password"], "pass");
        assert_eq!(value["waitUntil"], "networkidle");
        assert_eq!(value["captureTrace"], true);
        assert_eq!(value["captureScreenshot"], false);
        assert_eq!(value["captureConsole"], true);
        assert_eq!(value["captureNetwork"], false);
        assert_eq!(value["artifactDir"], "/tmp/aexeo-artifacts");
    }

    /// The spec must be base64 of UTF-8 JSON, not just base64: the runner
    /// decodes it with `Buffer.from(...).toString('utf8')` and then
    /// `JSON.parse`. A non-UTF-8 payload would decode to replacement
    /// characters and fail to parse inside Node.
    #[test]
    fn encoded_spec_is_base64_of_utf8_json() {
        use base64::Engine as _;
        let config = PlaywrightOwnedConfig::from_runtime(&Config::default().runtime());
        let encoded =
            encode_spec(&config.spec_for_url("https://ünïcode.example/")).expect("encode");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .expect("valid base64");
        let text = String::from_utf8(bytes).expect("decodes as utf-8");
        let value: serde_json::Value = serde_json::from_str(&text).expect("parses as json");
        assert_eq!(value["url"], "https://ünïcode.example/");
    }

    /// A runner that exits non-zero must produce a clear error rather than
    /// an empty fetch, so a broken browser install is diagnosable.
    #[test]
    fn failing_runner_surfaces_an_error() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runner = fake_runner(
            temp_dir.path(),
            "#!/bin/sh\necho 'browser failed to launch' >&2\nexit 1\n",
        );
        let result = fetch_with_playwright_using(
            Some(runner),
            "https://example.com",
            &Config::default().runtime(),
        );
        let error = result.expect_err("a failing runner must not yield a fetch result");
        let message = format!("{error:#}");
        assert!(
            message.contains("browser failed to launch"),
            "the runner's stderr must reach the user: {message}"
        );
    }

    /// A runner that prints nothing at all is a different failure from one
    /// that errors: there is no response to parse. It must not be reported
    /// as a successful empty fetch.
    #[test]
    fn silent_runner_is_not_reported_as_a_successful_fetch() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runner = fake_runner(temp_dir.path(), "#!/bin/sh\nexit 0\n");
        let result = fetch_with_playwright_using(
            Some(runner),
            "https://example.com",
            &Config::default().runtime(),
        );
        assert!(
            result.is_err(),
            "an empty response must be an error, not a 200 with no body"
        );
    }

    /// Malformed JSON on stdout is a contract violation between this crate
    /// and the runner, and must say so rather than panicking.
    #[test]
    fn malformed_runner_output_is_reported_clearly() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runner = fake_runner(temp_dir.path(), &json_runner("this is not json"));
        let result = fetch_with_playwright_using(
            Some(runner),
            "https://example.com",
            &Config::default().runtime(),
        );
        let error = result.expect_err("malformed output must not parse");
        assert!(!format!("{error:#}").is_empty());
    }

    /// Response headers come back as a flat map the crawler reads for
    /// `x-robots-tag` and content type, so they must survive the crossing.
    #[test]
    fn response_headers_survive_the_runner_contract() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runner = fake_runner(
            temp_dir.path(),
            &json_runner(
                r#"{"statusCode":200,"contentType":"text/html; charset=utf-8","body":"<html></html>","headers":{"x-robots-tag":"noindex","content-type":"text/html; charset=utf-8"},"effectiveUrl":"https://example.com/final"}"#,
            ),
        );
        let fetched = fetch_with_playwright_using(
            Some(runner),
            "https://example.com/redirected",
            &Config::default().runtime(),
        )
        .expect("parse response");
        assert_eq!(fetched.status_code, Some(200));
        assert_eq!(fetched.effective_url, "https://example.com/final");
        let headers = &fetched.headers;
        assert_eq!(
            headers.get("x-robots-tag").map(String::as_str),
            Some("noindex"),
            "a crawl decides noindex from this header; it must not be dropped"
        );
    }

    /// A non-200 from the page is a normal result, not an error: a 404 page
    /// is exactly what the crawler is looking for.
    #[test]
    fn non_success_status_is_returned_not_treated_as_an_error() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runner = fake_runner(
            temp_dir.path(),
            &json_runner(
                r#"{"statusCode":404,"contentType":"text/html","body":"<html>gone</html>","headers":{},"effectiveUrl":"https://example.com/missing"}"#,
            ),
        );
        let fetched = fetch_with_playwright_using(
            Some(runner),
            "https://example.com/missing",
            &Config::default().runtime(),
        )
        .expect("a 404 is a result, not a failure");
        assert_eq!(fetched.status_code, Some(404));
        assert!(fetched.body.unwrap_or_default().contains("gone"));
    }

    /// With an explicit runner, the probe answers "does this path exist",
    /// and says so in the message. It deliberately does not execute the
    /// runner: `doctor` must be cheap and side-effect-free, and a browser
    /// launch is neither.
    #[test]
    fn probe_with_an_explicit_runner_checks_existence_not_execution() {
        let temp_dir = tempfile::tempdir().unwrap();
        // Exits non-zero, so a probe that ran it would report unavailable.
        let runner = fake_runner(
            temp_dir.path(),
            "#!/bin/sh\necho 'Cannot find module playwright' >&2\nexit 1\n",
        );
        let doctor = probe_playwright_runtime_with(Some(runner.clone()));
        assert!(
            doctor.available,
            "an existing runner path is available: {doctor:?}"
        );
        assert_eq!(doctor.mode, "custom_runner");
        assert_eq!(
            doctor.executable.as_str(),
            runner.to_str().unwrap_or_default()
        );
        assert!(
            doctor.message.contains("exists"),
            "the message should say what was actually checked: {doctor:?}"
        );
    }

    /// Without an override, the probe really does run Node to find out, so it
    /// must always come back with a reason an operator can act on.
    #[test]
    fn probe_without_an_override_always_explains_itself() {
        let doctor = probe_playwright_runtime_with(None);
        assert!(
            !doctor.message.trim().is_empty(),
            "`doctor runtime` must always say what it found: {doctor:?}"
        );
        if doctor.available {
            assert_eq!(doctor.mode, "node_module");
        } else {
            assert!(
                !doctor.executable.trim().is_empty(),
                "an unavailable runtime should name the executable it tried: {doctor:?}"
            );
        }
    }

    #[test]
    fn probe_reports_available_for_a_working_runner() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runner = fake_runner(temp_dir.path(), "#!/bin/sh\nexit 0\n");
        let doctor = probe_playwright_runtime_with(Some(runner));
        assert!(doctor.available, "{doctor:?}");
    }

    /// A missing executable is distinct from a failing one, and the doctor
    /// must not try to spawn it and surface a raw OS error.
    #[test]
    fn probe_handles_a_nonexistent_runner() {
        let doctor = probe_playwright_runtime_with(Some(PathBuf::from(
            "/nonexistent/aexeo-playwright-runner",
        )));
        assert!(!doctor.available, "{doctor:?}");
        assert!(!doctor.message.trim().is_empty());
    }

    /// The only error treated as transient is `EINTR` raised while Node
    /// resolves the current working directory.
    ///
    /// That is a specific, narrow case: a signal can interrupt `uv_cwd` and
    /// there is nothing to fix, so the crawl can safely continue. It is not a
    /// general "the browser died" classifier — a closed browser or a refused
    /// connection must fail loudly, because retrying hides a real problem.
    #[test]
    fn only_eintr_while_resolving_the_working_directory_is_transient() {
        for message in [
            "uv_cwd EINTR",
            "Error: EINTR at process.cwd()",
            "operation interrupted (EINTR) while reading the current working directory",
        ] {
            assert!(
                is_transient_playwright_process_error(message),
                "{message:?} should be treated as transient"
            );
        }
        for message in [
            "Target page, context or browser has been closed",
            "net::ERR_CONNECTION_REFUSED at https://example.com",
            "playwright: executable doesn't exist",
            // EINTR alone is not enough without the cwd context.
            "read ECONNRESET",
        ] {
            assert!(
                !is_transient_playwright_process_error(message),
                "{message:?} must fail loudly rather than be retried"
            );
        }
    }
}
