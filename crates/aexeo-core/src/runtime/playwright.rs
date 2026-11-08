use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::hash::{BuildHasher, RandomState};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::egress::{CrawlEgress, FetchRefused};
use super::http::{FetchResult, redact_sensitive_headers};
use crate::config::RuntimeConfig;
use crate::link_graph::redact_url;

// Runners live in a private temp dir, so `playwright` is resolved from the
// working directory (or NODE_PATH) instead of from the runner's own location.
const PLAYWRIGHT_LOADER: &str = r#"
import { createRequire } from 'node:module';
import nodePath from 'node:path';

const { chromium } = createRequire(nodePath.join(process.cwd(), 'aexeo-runner.cjs'))('playwright');
"#;

const PLAYWRIGHT_RUNNER_HELPERS: &str = r#"
import dns from 'node:dns/promises';
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import readline from 'node:readline';

function decodeSpec(line) {
  return JSON.parse(Buffer.from(line.trim(), 'base64').toString('utf8'));
}

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

function normalizeCookies(cookies, defaultUrl) {
  return (cookies || [])
    .map((cookie) => {
      if (!cookie || typeof cookie !== 'object') return null;
      if (typeof cookie.name !== 'string' || typeof cookie.value !== 'string') return null;
      const hasDomain = typeof cookie.domain === 'string';
      return {
        name: cookie.name,
        value: cookie.value,
        url: hasDomain ? undefined : (typeof cookie.url === 'string' ? cookie.url : defaultUrl),
        domain: hasDomain ? cookie.domain : undefined,
        path: hasDomain ? (typeof cookie.path === 'string' ? cookie.path : '/') : undefined,
        httpOnly: Boolean(cookie.httpOnly),
        secure: Boolean(cookie.secure),
        expires: typeof cookie.expires === 'number' ? cookie.expires : undefined,
        sameSite: typeof cookie.sameSite === 'string' ? cookie.sameSite : undefined
      };
    })
    .filter(Boolean);
}

function originKey(urlString) {
  try {
    const url = new URL(urlString);
    const port = url.port || (url.protocol === 'https:' ? '443' : '80');
    const host = url.hostname.replace(/\.$/, '').replace(/^www\./, '').toLowerCase();
    return `${url.protocol}//${host}:${port}`;
  } catch {
    return null;
  }
}

const NON_PUBLIC = new net.BlockList();
for (const [address, prefix] of [
  ['0.0.0.0', 8], ['10.0.0.0', 8], ['100.64.0.0', 10], ['127.0.0.0', 8], ['169.254.0.0', 16],
  ['172.16.0.0', 12], ['192.0.0.0', 24], ['192.0.2.0', 24], ['192.168.0.0', 16], ['198.18.0.0', 15],
  ['198.51.100.0', 24], ['203.0.113.0', 24], ['224.0.0.0', 4], ['240.0.0.0', 4],
]) {
  NON_PUBLIC.addSubnet(address, prefix, 'ipv4');
}
for (const [address, prefix] of [
  ['::', 128], ['::1', 128], ['fc00::', 7], ['fe80::', 10], ['fec0::', 10], ['ff00::', 8], ['2001:db8::', 32],
]) {
  NON_PUBLIC.addSubnet(address, prefix, 'ipv6');
}

const hostVerdicts = new Map();

function hostIsNonPublic(urlString) {
  let hostname;
  try {
    const url = new URL(urlString);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return Promise.resolve(false);
    hostname = url.hostname.replace(/^\[|\]$/g, '');
  } catch {
    return Promise.resolve(false);
  }
  if (!hostVerdicts.has(hostname)) {
    const lookup = net.isIP(hostname)
      ? Promise.resolve([{ address: hostname, family: net.isIP(hostname) }])
      : dns.lookup(hostname, { all: true }).catch(() => []);
    hostVerdicts.set(
      hostname,
      lookup.then((answers) =>
        answers.some(({ address, family }) => NON_PUBLIC.check(address, family === 6 ? 'ipv6' : 'ipv4'))
      )
    );
  }
  return hostVerdicts.get(hostname);
}

// CDP Fetch rather than page.route: route handlers see only the first URL of a
// redirect chain and Playwright re-applies their header overrides to later
// hops, which would carry credentials off-origin. Fetch pauses every hop, and
// its overrides do not extend past the hop they were given on.
async function installEgressGuard(context, page, spec) {
  const credentials = { ...(spec.headers || {}) };
  if (spec.basicAuth && spec.basicAuth.username && spec.basicAuth.password) {
    const pair = `${spec.basicAuth.username}:${spec.basicAuth.password}`;
    credentials.Authorization = `Basic ${Buffer.from(pair).toString('base64')}`;
  }
  const guard = { blockedDocument: null };
  if (Object.keys(credentials).length === 0 && spec.allowPrivateNetwork) {
    return guard;
  }
  const crawlOrigin = originKey(spec.crawlOrigin);
  const session = await context.newCDPSession(page);
  session.on('Fetch.requestPaused', async ({ requestId, request, resourceType }) => {
    try {
      if (!spec.allowPrivateNetwork && (await hostIsNonPublic(request.url))) {
        if (resourceType === 'Document') guard.blockedDocument = request.url;
        await session.send('Fetch.failRequest', { requestId, errorReason: 'BlockedByClient' });
        return;
      }
      const params = { requestId };
      if (crawlOrigin !== null && originKey(request.url) === crawlOrigin) {
        const merged = { ...request.headers, ...credentials };
        params.headers = Object.entries(merged).map(([name, value]) => ({ name, value: String(value) }));
      }
      await session.send('Fetch.continueRequest', params);
    } catch {
      // The target closed or navigated away; nothing is left to continue.
    }
  });
  await session.send('Fetch.enable', { patterns: [{ urlPattern: '*' }] });
  return guard;
}

function redactUrl(urlString) {
  try {
    const url = new URL(urlString);
    url.username = '';
    url.password = '';
    url.search = '';
    url.hash = '';
    return url.toString();
  } catch {
    return '[invalid URL]';
  }
}

// Only HTTP redirects belonging to the final navigation are recorded. A
// client-side navigation after goto is not evidence of an HTTP redirect chain.
async function navigationRedirectChain(response, effectiveUrl) {
  if (!response || response.url() !== effectiveUrl) return null;
  const hops = [];
  let request = response.request().redirectedFrom();
  while (request) {
    let prior;
    try { prior = await request.response(); } catch { return null; }
    if (!prior || prior.status() < 300 || prior.status() >= 400) return null;
    hops.push({ url: redactUrl(request.url()), status_code: prior.status() });
    request = request.redirectedFrom();
  }
  return hops.reverse();
}

function fetchFailure(guard, error) {
  if (guard.blockedDocument) {
    const blocked = redactUrl(guard.blockedDocument);
    guard.blockedDocument = null;
    return `refused to fetch ${blocked}: it resolves to a non-public network address and the crawl target does not; set \`[runtime] allow_private_network = true\` if this target is intended`;
  }
  // Playwright quotes the URL it failed on, secrets included.
  return String(error).replace(/https?:\/\/[^\s"'<>]+/g, redactUrl);
}
"#;

const PLAYWRIGHT_INLINE_RUNNER: &str = r#"
let encodedSpec = '';
for await (const chunk of process.stdin) encodedSpec += chunk;
const spec = decodeSpec(encodedSpec);

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext();
const cookies = normalizeCookies(spec.cookies, spec.crawlOrigin);
if (cookies.length > 0) {
  await context.addCookies(cookies);
}

const page = await context.newPage();
const guard = await installEgressGuard(context, page, spec);
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
      url: redactUrl(response.url()),
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
    fs.mkdirSync(artifactDir, { recursive: true, mode: 0o700 });
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
    redirectChain: await navigationRedirectChain(mainResponse, effectiveUrl),
    artifacts: {
      screenshotPath,
      tracePath,
      consolePath,
      networkPath
    }
  };
  process.stdout.write(JSON.stringify(payload));
} catch (error) {
  console.error(fetchFailure(guard, error));
  process.exitCode = 1;
} finally {
  await context.close();
  await browser.close();
}
"#;

const PLAYWRIGHT_SESSION_RUNNER: &str = r#"
const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const lines = rl[Symbol.asyncIterator]();
const baseSpec = decodeSpec((await lines.next()).value);

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext();
const initialCookies = normalizeCookies(baseSpec.cookies, baseSpec.crawlOrigin);
if (initialCookies.length > 0) {
  await context.addCookies(initialCookies);
}
const page = await context.newPage();
const guard = await installEgressGuard(context, page, baseSpec);

async function fetchUrl(url) {
  const consoleEvents = [];
  const networkEvents = [];
  const consoleListener = (message) => {
    consoleEvents.push({ type: message.type(), text: message.text() });
  };
  const networkListener = async (response) => {
    networkEvents.push({
      url: redactUrl(response.url()),
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

  guard.blockedDocument = null;
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
      fs.mkdirSync(artifactDir, { recursive: true, mode: 0o700 });
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
      redirectChain: await navigationRedirectChain(mainResponse, effectiveUrl),
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
      error: fetchFailure(guard, error)
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

try {
  for (let next = await lines.next(); !next.done; next = await lines.next()) {
    const line = next.value;
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
    crawl_origin: &'a str,
    allow_private_network: bool,
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
    // Removed on drop, so it outlives the child that loaded the runner from it.
    _runner: MaterializedRunner,
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
    crawl_origin: String,
    allow_private_network: bool,
}

/// A Node runner written to a private directory. The source is embedded in the
/// binary because an installed binary has no checkout to write into, and the
/// directory is private because Node will execute whatever is at this path.
#[derive(Debug)]
struct MaterializedRunner {
    dir: PathBuf,
    script: PathBuf,
}

impl MaterializedRunner {
    fn write(parts: &[&str]) -> Result<Self> {
        let dir = private_runner_dir()?;
        let script = dir.join("aexeo-playwright-runner.mjs");
        let runner = Self { dir, script };
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options
            .open(&runner.script)
            .and_then(|mut file| file.write_all(parts.concat().as_bytes()))
            .with_context(|| {
                format!(
                    "failed to write playwright runner {}",
                    runner.script.display()
                )
            })?;
        Ok(runner)
    }
}

impl Drop for MaterializedRunner {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn private_runner_dir() -> Result<PathBuf> {
    let parent = std::env::temp_dir();
    for attempt in 0..16u32 {
        // Exclusive creation is the safeguard against a planted path; the name
        // only has to be hard to guess, and `RandomState` is seeded randomly.
        let suffix = RandomState::new().hash_one((std::process::id(), attempt));
        let dir = parent.join(format!("aexeo-playwright-{suffix:016x}"));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to create playwright runner dir in {}",
                        parent.display()
                    )
                });
            }
        }
    }
    bail!(
        "could not create a playwright runner dir in {}",
        parent.display()
    )
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
        "artifactDir": spec.artifact_dir,
        "crawlOrigin": spec.crawl_origin,
        "allowPrivateNetwork": spec.allow_private_network
    }))?))
}

pub(crate) fn playwright_is_available() -> bool {
    probe_playwright_runtime().available
}

pub(crate) fn probe_playwright_runtime() -> PlaywrightDoctor {
    probe_playwright_runtime_with(configured_playwright_executable())
}

impl PlaywrightFetcher {
    pub(crate) fn new(runtime: &RuntimeConfig<'_>, egress: &CrawlEgress) -> Result<Self> {
        let config = PlaywrightOwnedConfig::from_runtime(runtime, egress);
        if let Some(executable) = configured_playwright_executable() {
            return Ok(Self::OneShot(executable, config));
        }
        Ok(Self::Session(PlaywrightSession::spawn_from_config(config)?))
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
    fn from_runtime(runtime: &RuntimeConfig<'_>, egress: &CrawlEgress) -> Self {
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
            crawl_origin: egress.origin().to_string(),
            allow_private_network: egress.allow_private(),
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
            crawl_origin: &self.crawl_origin,
            allow_private_network: self.allow_private_network,
        }
    }
}

impl PlaywrightSession {
    fn spawn_from_config(config: PlaywrightOwnedConfig) -> Result<Self> {
        let encoded_spec = encode_spec(&config.spec_for_url(""))?;
        let runner = MaterializedRunner::write(&[
            PLAYWRIGHT_LOADER,
            PLAYWRIGHT_RUNNER_HELPERS,
            PLAYWRIGHT_SESSION_RUNNER,
        ])?;
        let mut child = Command::new("node")
            .arg(&runner.script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn persistent playwright runtime")?;
        let mut stdin = child
            .stdin
            .take()
            .context("playwright session stdin unavailable")?;
        // Credentials travel on stdin: argv is readable by other local users.
        stdin
            .write_all(encoded_spec.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush())
            .context("failed to send spec to playwright runtime")?;
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
            _runner: runner,
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
            bail!(
                "failed to send playwright request for {}: {error}; {stderr}",
                redact_url(url)
            );
        }
        if let Err(error) = self.stdin.write_all(b"\n") {
            let stderr = self.read_stderr();
            bail!(
                "failed to terminate playwright request for {}: {error}; {stderr}",
                redact_url(url)
            );
        }
        if let Err(error) = self.stdin.flush() {
            let stderr = self.read_stderr();
            bail!(
                "failed to flush playwright request for {}: {error}; {stderr}",
                redact_url(url)
            );
        }

        let mut line = String::new();
        let read = self.stdout.read_line(&mut line)?;
        if read == 0 {
            let stderr = self.read_stderr();
            bail!(
                "playwright runtime terminated while fetching {}: {stderr}",
                redact_url(url)
            );
        }
        let payload: serde_json::Value =
            serde_json::from_str(line.trim_end()).with_context(|| {
                format!(
                    "invalid persistent playwright response for {}",
                    redact_url(url)
                )
            })?;
        if !payload["ok"].as_bool().unwrap_or(false) {
            let error = payload["error"].as_str().unwrap_or("unknown error");
            if let Some(refused) = runner_refusal(error) {
                return Err(refused.into());
            }
            bail!("playwright runtime failed for {}: {error}", redact_url(url));
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
    let runner = MaterializedRunner::write(&[PLAYWRIGHT_LOADER, PLAYWRIGHT_PROBE_RUNNER])
        .map_err(std::io::Error::other)?;
    Command::new("node").arg(&runner.script).output()
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
    let origin = super::http::origin_for_url(url);
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
            crawl_origin: &origin,
            allow_private_network: true,
        },
    )
}

/// The runner reports a blocked navigation in this form, with its URL redacted.
fn runner_refusal(message: &str) -> Option<FetchRefused> {
    let message = message.trim();
    message
        .starts_with("refused to fetch ")
        .then(|| FetchRefused(message.to_string()))
}

fn fetch_with_playwright_spec(
    executable: Option<PathBuf>,
    spec: &PlaywrightSpec<'_>,
) -> Result<FetchResult> {
    let encoded_spec = encode_spec(spec)?;
    // Held until the child has exited: the child loads the runner from here.
    let mut runner = None;
    let mut command = if let Some(executable) = executable.as_ref() {
        Command::new(executable)
    } else {
        let written = MaterializedRunner::write(&[
            PLAYWRIGHT_LOADER,
            PLAYWRIGHT_RUNNER_HELPERS,
            PLAYWRIGHT_INLINE_RUNNER,
        ])?;
        let mut command = Command::new("node");
        command.arg(&written.script);
        runner = Some(written);
        command
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().with_context(|| {
        format!(
            "failed to spawn playwright runtime for {}",
            redact_url(spec.url)
        )
    })?;
    // Credentials travel on stdin: argv is readable by other local users. On a
    // thread so a child that never reads cannot block the wait below, and a
    // write error is ignored because a runner may legitimately ignore its spec.
    let stdin = child.stdin.take();
    thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(encoded_spec.as_bytes());
        }
    });
    let output = child.wait_with_output().with_context(|| {
        format!(
            "failed to execute playwright runtime for {}",
            redact_url(spec.url)
        )
    })?;
    drop(runner);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if let Some(refused) = runner_refusal(&stderr) {
            return Err(refused.into());
        }
        bail!(
            "playwright runtime failed for {}: {stderr}",
            redact_url(spec.url)
        );
    }
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "invalid playwright runtime response for {}",
            redact_url(spec.url)
        )
    })?;
    let mut result = fetch_result_from_payload(spec.url, &payload);
    // Custom runners can provide useful hops, but cannot attest the built-in
    // runner’s observation contract. Keep coverage conservative.
    if executable.is_some() {
        result.redirect_chain_observed = false;
    }
    Ok(result)
}

fn fetch_result_from_payload(url: &str, payload: &serde_json::Value) -> FetchResult {
    let mut headers = payload["headers"]
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
    redact_sensitive_headers(&mut headers);
    let redirect_chain = payload["redirectChain"].as_array().and_then(|hops| {
        hops.iter()
            .map(|hop| {
                let url = hop["url"].as_str()?;
                let parsed = url::Url::parse(url).ok()?;
                if !matches!(parsed.scheme(), "http" | "https") {
                    return None;
                }
                let status = hop["status_code"].as_u64()?;
                if !(300..400).contains(&status) {
                    return None;
                }
                Some(aexeo_contracts::RedirectHop {
                    url: redact_url(url),
                    status_code: status as u16,
                })
            })
            .collect::<Option<Vec<_>>>()
    });
    FetchResult {
        status_code: payload["statusCode"].as_u64().map(|value| value as u16),
        content_type: payload["contentType"].as_str().map(str::to_string),
        body: payload["body"].as_str().map(str::to_string),
        headers,
        effective_url: payload["effectiveUrl"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| url.to_string()),
        effective_url_observed: payload["effectiveUrl"]
            .as_str()
            .is_some_and(|url| !url.is_empty()),
        headers_observed: payload["headers"]
            .as_object()
            .is_some_and(|headers| headers.values().all(serde_json::Value::is_string)),
        redirect_chain_observed: redirect_chain.is_some(),
        redirect_chain: redirect_chain.unwrap_or_default(),
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
    fn browser_redirect_provenance_is_redacted_and_rejects_partial_chains() {
        let payload = serde_json::json!({"redirectChain":[
            {"url":"https://user:secret@example.org/old?token=secret#part","status_code":301},
            {"url":"https://example.org/next?private=1","status_code":302}
        ]});
        let fetched = super::fetch_result_from_payload("https://example.org/old", &payload);
        assert!(fetched.redirect_chain_observed);
        assert_eq!(fetched.redirect_chain.len(), 2);
        assert_eq!(fetched.redirect_chain[0].url, "https://example.org/old");
        assert_eq!(fetched.redirect_chain[1].url, "https://example.org/next");
        let invalid = serde_json::json!({"redirectChain":[
            {"url":"https://example.org/old","status_code":301},
            {"url":"https://example.org/next","status_code":200}
        ]});
        assert!(
            !super::fetch_result_from_payload("https://example.org/old", &invalid)
                .redirect_chain_observed
        );
        assert!(
            super::fetch_result_from_payload("https://example.org/old", &serde_json::json!({}))
                .redirect_chain
                .is_empty()
        );
    }

    #[test]
    fn missing_browser_response_metadata_is_not_treated_as_observed() {
        let fetched = super::fetch_result_from_payload(
            "https://example.com",
            &serde_json::json!({"headers": {"x-robots-tag": null}}),
        );
        assert_eq!(fetched.effective_url, "https://example.com");
        assert!(!fetched.effective_url_observed);
        assert!(!fetched.headers_observed);
        assert_eq!(fetched.status_code, None);
    }

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
            crawl_origin: "https://example.com/".to_string(),
            allow_private_network: false,
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
        assert_eq!(value["crawlOrigin"], "https://example.com/");
        assert_eq!(value["allowPrivateNetwork"], false);
    }

    /// The spec must be base64 of UTF-8 JSON, not just base64: the runner
    /// decodes it with `Buffer.from(...).toString('utf8')` and then
    /// `JSON.parse`. A non-UTF-8 payload would decode to replacement
    /// characters and fail to parse inside Node.
    #[test]
    fn encoded_spec_is_base64_of_utf8_json() {
        use base64::Engine as _;
        let egress = crate::runtime::egress::CrawlEgress::new("https://example.com/", false);
        let config = PlaywrightOwnedConfig::from_runtime(&Config::default().runtime(), &egress);
        let encoded =
            encode_spec(&config.spec_for_url("https://ünïcode.example/")).expect("encode");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .expect("valid base64");
        let text = String::from_utf8(bytes).expect("decodes as utf-8");
        let value: serde_json::Value = serde_json::from_str(&text).expect("parses as json");
        assert_eq!(value["url"], "https://ünïcode.example/");
    }

    /// Set-cookie must not reach the snapshot, which is persisted to
    /// checkpoints and audit artifacts.
    #[test]
    fn browser_response_headers_are_redacted() {
        let fetched = super::fetch_result_from_payload(
            "https://example.com",
            &serde_json::json!({"headers": {"Set-Cookie": "session=abc", "x-robots-tag": "noindex"}}),
        );
        assert_eq!(fetched.headers["set-cookie"], "[redacted]");
        assert_eq!(fetched.headers["x-robots-tag"], "noindex");
    }

    /// Credentials go over stdin, never argv, where other local users can read
    /// them. The fake runner proves both halves at once.
    #[test]
    fn custom_runner_receives_the_spec_on_stdin_not_argv() {
        let temp_dir = tempfile::tempdir().unwrap();
        let stdin_capture = temp_dir.path().join("stdin");
        let argv_capture = temp_dir.path().join("argv");
        let runner = fake_runner(
            temp_dir.path(),
            &format!(
                "#!/bin/sh\nprintf '%s' \"$*\" > '{}'\ncat > '{}'\nprintf '%s' '{{\"statusCode\":200,\"body\":\"\",\"headers\":{{}}}}'\n",
                argv_capture.display(),
                stdin_capture.display(),
            ),
        );
        let basic_auth = BTreeMap::from([
            ("username".to_string(), "user".to_string()),
            ("password".to_string(), "hunter2".to_string()),
        ]);
        let config = Config {
            crawl_basic_auth: basic_auth,
            ..Config::default()
        };
        fetch_with_playwright_using(Some(runner), "https://example.com", &config.runtime())
            .expect("runner succeeds");

        assert_eq!(fs::read_to_string(&argv_capture).unwrap(), "");
        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(fs::read_to_string(&stdin_capture).unwrap().trim())
            .unwrap();
        let spec: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(spec["basicAuth"]["password"], "hunter2");
    }

    /// The runner must not be written under a compile-time path: an installed
    /// binary has no checkout there. It must also not be shared or readable,
    /// because Node executes it.
    #[test]
    fn runner_is_materialized_in_a_private_temp_dir_and_removed() {
        let runner = super::MaterializedRunner::write(&["// source"]).unwrap();
        assert!(runner.script.starts_with(std::env::temp_dir()));
        let mode = fs::metadata(&runner.dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        let mode = fs::metadata(&runner.script).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let other = super::MaterializedRunner::write(&["// source"]).unwrap();
        assert_ne!(runner.dir, other.dir);
        let dir = runner.dir.clone();
        drop(runner);
        assert!(!dir.exists());
    }

    /// Minimal server for the real-browser tests: records request heads.
    fn recording_server(
        respond: impl Fn(&str) -> String + Send + 'static,
    ) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = requests.clone();
        // Detached: the thread ends with the test process.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut buffer = [0_u8; 4096];
                while !head.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(size) if size > 0 => head.extend_from_slice(&buffer[..size]),
                        _ => break,
                    }
                }
                let head = String::from_utf8_lossy(&head).to_lowercase();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                recorded.lock().unwrap().push(head);
                let _ = stream.write_all(respond(&path).as_bytes());
            }
        });
        (port, requests)
    }

    fn html(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn found(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    fn browser_config(origin: &str, allow_private: bool) -> PlaywrightOwnedConfig {
        let mut config = Config::default();
        config
            .crawl_headers
            .insert("x-secret".to_string(), "header-secret".to_string());
        config
            .crawl_basic_auth
            .insert("username".to_string(), "user".to_string());
        config
            .crawl_basic_auth
            .insert("password".to_string(), "pass".to_string());
        let egress = crate::runtime::egress::CrawlEgress::new(origin, allow_private);
        PlaywrightOwnedConfig::from_runtime(&config.runtime(), &egress)
    }

    /// Real Chromium, skipped where Playwright is not installed. Covers what a
    /// fake runner cannot: a redirect hop and a third-party subresource must
    /// not see the credentials, while same-origin hops must.
    #[test]
    fn browser_sends_credentials_only_to_the_crawl_origin() {
        if !probe_playwright_runtime_with(None).available {
            eprintln!("skipped: Playwright is not installed");
            return;
        }
        let (other_port, other) = recording_server(|_| html("<h1>other</h1>"));
        let (crawl_port, crawl) = recording_server(move |path| match path {
            "/" => found("/same-origin"),
            "/same-origin" => html(&format!(
                "<img src=\"http://localhost:{other_port}/pixel.png\"><h1>home</h1>"
            )),
            "/leaves" => found(&format!("http://localhost:{other_port}/landing")),
            _ => html(""),
        });
        let origin = format!("http://127.0.0.1:{crawl_port}");
        let mut session =
            super::PlaywrightSession::spawn_from_config(browser_config(&origin, true)).unwrap();

        let fetched = session.fetch(&format!("{origin}/")).unwrap();
        assert_eq!(fetched.effective_url, format!("{origin}/same-origin"));
        let left = session.fetch(&format!("{origin}/leaves")).unwrap();
        assert_eq!(
            left.effective_url,
            format!("http://localhost:{other_port}/landing")
        );
        drop(session);

        let crawl = crawl.lock().unwrap().clone();
        let other = other.lock().unwrap().clone();
        for path in ["get / ", "get /same-origin ", "get /leaves "] {
            let request = crawl
                .iter()
                .find(|request| request.starts_with(path))
                .unwrap_or_else(|| panic!("{path} not requested: {crawl:?}"));
            assert!(request.contains("x-secret: header-secret"), "{request}");
            assert!(request.contains("authorization: basic"), "{request}");
        }
        assert!(
            other.iter().any(|r| r.starts_with("get /pixel.png ")),
            "{other:?}"
        );
        assert!(
            other.iter().any(|r| r.starts_with("get /landing ")),
            "{other:?}"
        );
        for request in &other {
            assert!(!request.contains("x-secret"), "{request}");
            assert!(!request.contains("authorization"), "{request}");
        }
    }

    #[test]
    fn browser_records_http_redirects_in_both_runner_modes() {
        if !probe_playwright_runtime_with(None).available {
            eprintln!("skipped: Playwright is not installed");
            return;
        }
        let (port, _) = recording_server(|path| match path {
            "/old?token=secret" => found("/middle?token=other"),
            "/middle?token=other" => found("/final"),
            _ => html("<h1>done</h1>"),
        });
        let origin = format!("http://127.0.0.1:{port}");
        let url = format!("{origin}/old?token=secret");
        let mut session =
            super::PlaywrightSession::spawn_from_config(browser_config(&origin, true)).unwrap();
        let session_result = session.fetch(&url).unwrap();
        drop(session);
        let inline_result =
            fetch_with_playwright_using(None, &url, &Config::default().runtime()).unwrap();
        for fetched in [session_result, inline_result] {
            assert_eq!(fetched.effective_url, format!("{origin}/final"));
            let hops: Vec<_> = fetched
                .redirect_chain
                .iter()
                .map(|hop| (hop.url.as_str(), hop.status_code))
                .collect();
            assert_eq!(
                hops,
                vec![
                    (format!("{origin}/old").as_str(), 302),
                    (format!("{origin}/middle").as_str(), 302)
                ]
            );
        }
    }

    #[test]
    fn browser_refuses_private_targets_under_a_public_crawl() {
        if !probe_playwright_runtime_with(None).available {
            eprintln!("skipped: Playwright is not installed");
            return;
        }
        let (port, requests) = recording_server(|_| html("<h1>private</h1>"));
        let mut session = super::PlaywrightSession::spawn_from_config(browser_config(
            "https://example.com/",
            false,
        ))
        .unwrap();
        for url in [
            format!("http://127.0.0.1:{port}/?token=hunter2"),
            format!("http://localhost:{port}/?token=hunter2"),
        ] {
            let error = session.fetch(&url).expect_err(&url);
            assert!(crate::runtime::egress::is_refused(&error), "{error:#}");
            let message = format!("{error:#}");
            assert!(message.contains("non-public"), "{message}");
            assert!(!message.contains("hunter2"), "{message}");
        }
        drop(session);
        assert!(requests.lock().unwrap().is_empty());
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
