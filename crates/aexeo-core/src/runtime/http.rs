use anyhow::{Context, Result};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use reqwest::{Method, StatusCode};
use std::collections::BTreeMap;
use std::error::Error as _;
use std::io::Read;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use url::Url;

use super::egress::{CrawlEgress, FetchRefused, is_refused};
use super::fetcher::FetchOutcome;
use crate::link_graph::redact_url;

#[derive(Debug, Clone)]
pub(crate) struct FetchResult {
    pub(crate) status_code: Option<u16>,
    pub(crate) content_type: Option<String>,
    pub(crate) body: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) effective_url: String,
    pub(crate) effective_url_observed: bool,
    pub(crate) headers_observed: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct HttpFetcher {
    client: Client,
    headers: HeaderMap,
    cookies: Vec<BTreeMap<String, String>>,
    basic_auth: BTreeMap<String, String>,
    egress: CrawlEgress,
    max_response_bytes: u64,
}

const MAX_REDIRECTS: usize = 10;
const REDACTED_HEADER_VALUE: &str = "[redacted]";

pub(crate) fn normalize_base_url(base_url: &str) -> String {
    format!("{}/", base_url.trim_end_matches('/'))
}

pub(crate) fn origin_for_url(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| {
            let scheme = parsed.scheme();
            let host = parsed.host_str()?;
            let port = parsed
                .port()
                .map(|value| format!(":{value}"))
                .unwrap_or_default();
            Some(format!("{scheme}://{host}{port}/"))
        })
        .unwrap_or_else(|| normalize_base_url(url))
}

pub(crate) fn host_for_url(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| {
            let host = parsed.host_str()?.to_string();
            let port = parsed
                .port()
                .map(|value| format!(":{value}"))
                .unwrap_or_default();
            Some(format!("{host}{port}"))
        })
        .unwrap_or_default()
}

fn comparable_host(host: &str) -> String {
    let without_userinfo = host.rsplit_once('@').map(|(_, rest)| rest).unwrap_or(host);
    let without_port = if without_userinfo.starts_with('[') {
        without_userinfo
            .split_once(']')
            .map(|(value, _)| value)
            .unwrap_or(without_userinfo)
    } else {
        without_userinfo
            .split_once(':')
            .map(|(value, _)| value)
            .unwrap_or(without_userinfo)
    };
    let normalized = without_port.trim_end_matches('.').to_ascii_lowercase();
    normalized
        .strip_prefix("www.")
        .unwrap_or(&normalized)
        .to_string()
}

pub(crate) fn same_site_host(left: &str, right: &str) -> bool {
    comparable_host(left) == comparable_host(right)
}

/// Whether two URLs are the same crawl origin: scheme, host and port, treating
/// `www.` as equivalent only when scheme and port match.
pub(crate) fn same_crawl_origin(left: &str, right: &str) -> bool {
    let (Ok(left), Ok(right)) = (Url::parse(left), Url::parse(right)) else {
        return false;
    };
    let (Some(left_host), Some(right_host)) = (left.host_str(), right.host_str()) else {
        return false;
    };
    left.scheme() == right.scheme()
        && left.port_or_known_default() == right.port_or_known_default()
        && same_site_host(left_host, right_host)
}

fn headers_from_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap> {
    let mut header_map = HeaderMap::new();
    for (key, value) in headers {
        let header_name = HeaderName::from_bytes(key.as_bytes())
            .with_context(|| format!("invalid header name '{key}'"))?;
        let header_value = HeaderValue::from_str(value)
            .with_context(|| format!("invalid header value for '{key}'"))?;
        header_map.insert(header_name, header_value);
    }
    Ok(header_map)
}

fn cookie_header_value(cookies: &[BTreeMap<String, String>]) -> Option<String> {
    let pairs = cookies
        .iter()
        .flat_map(|cookie| cookie.iter())
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>();
    if pairs.is_empty() {
        None
    } else {
        Some(pairs.join("; "))
    }
}

impl HttpFetcher {
    pub(crate) fn new(
        runtime: &crate::config::RuntimeConfig<'_>,
        egress: &CrawlEgress,
    ) -> Result<Self> {
        // Redirects are followed by hand in `send` because credentials and
        // policy are per hop: a reqwest redirect policy can only follow or
        // stop, not strip headers from the next request.
        let client = Client::builder()
            .redirect(Policy::none())
            .dns_resolver(egress.resolver())
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            headers: headers_from_map(runtime.crawl_headers)?,
            cookies: runtime.crawl_cookies.to_vec(),
            basic_auth: runtime.crawl_basic_auth.clone(),
            egress: egress.clone(),
            max_response_bytes: runtime.crawl_max_response_bytes,
        })
    }

    fn send(
        &self,
        method: Method,
        url: &str,
        credentials: bool,
        customize: &dyn Fn(RequestBuilder) -> RequestBuilder,
    ) -> Result<Response> {
        let mut current =
            Url::parse(url).with_context(|| format!("failed to fetch URL: {}", redact_url(url)))?;
        for _ in 0..=MAX_REDIRECTS {
            self.egress.check_url(&current)?;
            let mut request = customize(self.client.request(method.clone(), current.clone()));
            if credentials && self.egress.allows_credentials(&current) {
                request = apply_runtime_credentials(
                    request.headers(self.headers.clone()),
                    &self.cookies,
                    &self.basic_auth,
                );
            }
            let response = request
                .send()
                .map_err(|error| send_error(error, &current))?;
            match redirect_target(&response, &current) {
                Some(next) => current = next,
                None => return Ok(response),
            }
        }
        Err(FetchRefused(format!(
            "refused to fetch {}: more than {MAX_REDIRECTS} redirects",
            redact_url(url)
        ))
        .into())
    }

    pub(crate) fn fetch(&self, url: &str) -> Result<FetchResult> {
        self.fetch_customized(url, &|request| request)
    }

    pub(crate) fn fetch_with_timeout(&self, url: &str, timeout: Duration) -> Result<FetchResult> {
        self.fetch_customized(url, &|request| request.timeout(timeout))
    }

    /// Status of a HEAD request carrying the crawl credentials where allowed.
    pub(crate) fn head_status(&self, url: &str, timeout: Duration) -> Result<StatusCode> {
        let response = self.send(Method::HEAD, url, true, &|request| request.timeout(timeout))?;
        Ok(response.status())
    }

    /// Status and content type of a GET with this `Accept`, sent without
    /// credentials and without reading the body.
    pub(crate) fn negotiate(
        &self,
        url: &str,
        accept: &str,
        timeout: Duration,
    ) -> Result<(StatusCode, Option<String>)> {
        let response = self.send(Method::GET, url, false, &|request| {
            request
                .header(reqwest::header::ACCEPT, accept)
                .timeout(timeout)
        })?;
        Ok((response.status(), content_type_of(&response)))
    }

    fn fetch_customized(
        &self,
        url: &str,
        customize: &dyn Fn(RequestBuilder) -> RequestBuilder,
    ) -> Result<FetchResult> {
        let response = self.send(Method::GET, url, true, customize)?;
        let status_code = Some(response.status().as_u16());
        let effective_url = response.url().to_string();
        let content_type = content_type_of(&response);
        let headers_observed = response
            .headers()
            .values()
            .all(|value| value.to_str().is_ok());
        let headers = response_headers(response.headers());
        let body = read_capped_body(response, content_type.as_deref(), self.max_response_bytes)?;
        Ok(FetchResult {
            status_code,
            content_type,
            body,
            headers,
            effective_url,
            effective_url_observed: true,
            headers_observed,
        })
    }

    pub(crate) fn fetch_batch(
        &self,
        urls: &[String],
        fetch_retry_budget: usize,
    ) -> Vec<FetchOutcome> {
        let (sender, receiver) = mpsc::channel();
        for (index, url) in urls.iter().cloned().enumerate() {
            let sender = sender.clone();
            let fetcher = self.clone();
            thread::spawn(move || {
                let started_at = std::time::Instant::now();
                let mut retries = 0usize;
                let result = loop {
                    match fetcher.fetch(&url) {
                        Ok(fetched) => break Ok(fetched),
                        Err(error) if retries < fetch_retry_budget && !is_refused(&error) => {
                            retries += 1;
                        }
                        Err(error) => break Err(error),
                    }
                };
                let _ = sender.send((
                    index,
                    FetchOutcome {
                        url,
                        result,
                        retries,
                        elapsed_us: started_at.elapsed().as_micros() as u64,
                    },
                ));
            });
        }
        drop(sender);
        let mut outcomes = Vec::with_capacity(urls.len());
        while let Ok((index, outcome)) = receiver.recv() {
            outcomes.push((index, outcome));
            if outcomes.len() == urls.len() {
                break;
            }
        }
        outcomes.sort_by_key(|(index, _)| *index);
        outcomes.into_iter().map(|(_, outcome)| outcome).collect()
    }
}

fn apply_runtime_credentials(
    request: RequestBuilder,
    cookies: &[BTreeMap<String, String>],
    basic_auth: &BTreeMap<String, String>,
) -> RequestBuilder {
    let request = if let Some(cookie_header) = cookie_header_value(cookies) {
        request.header(reqwest::header::COOKIE, cookie_header)
    } else {
        request
    };
    if let (Some(username), Some(password)) =
        (basic_auth.get("username"), basic_auth.get("password"))
    {
        request.basic_auth(username, Some(password))
    } else {
        request
    }
}

/// Surface a policy refusal raised inside the resolver; reqwest wraps it in
/// several layers that would otherwise hide it behind a generic send error.
fn send_error(error: reqwest::Error, url: &Url) -> anyhow::Error {
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(refused) = cause.downcast_ref::<FetchRefused>() {
            return refused.clone().into();
        }
        source = cause.source();
    }
    // reqwest's own message repeats the full URL, query included.
    anyhow::Error::new(error.without_url())
        .context(format!("failed to fetch URL: {}", redact_url(url.as_str())))
}

fn redirect_target(response: &Response, current: &Url) -> Option<Url> {
    let status = response.status().as_u16();
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let location = response.headers().get(reqwest::header::LOCATION)?;
    current.join(location.to_str().ok()?).ok()
}

fn content_type_of(response: &Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// Read at most `max_bytes` of the body. An oversized body is a refusal; a
/// transport failure mid-body keeps its `None` meaning for the caller.
fn read_capped_body(
    response: Response,
    content_type: Option<&str>,
    max_bytes: u64,
) -> Result<Option<String>> {
    let url = redact_url(response.url().as_str());
    let oversized = || {
        FetchRefused(format!(
            "refused to read {url}: response body exceeds crawl_max_response_bytes ({max_bytes} bytes)"
        ))
    };
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(oversized().into());
    }
    let mut bytes = Vec::new();
    if response
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Ok(None);
    }
    if bytes.len() as u64 > max_bytes {
        return Err(oversized().into());
    }
    Ok(Some(decode_body(&bytes, content_type)))
}

fn decode_body(bytes: &[u8], content_type: Option<&str>) -> String {
    let encoding = content_type
        .and_then(|value| {
            value.split(';').skip(1).find_map(|parameter| {
                let (name, label) = parameter.split_once('=')?;
                name.trim()
                    .eq_ignore_ascii_case("charset")
                    .then(|| label.trim().trim_matches('"'))
            })
        })
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    encoding.decode(bytes).0.into_owned()
}

/// Response headers are persisted to checkpoints and audit artifacts, so
/// values that can carry session material are replaced at capture. The name
/// stays so a rule can still see that the header was present.
pub(crate) fn redact_sensitive_headers(headers: &mut BTreeMap<String, String>) {
    for (name, value) in headers.iter_mut() {
        if is_sensitive_header(name) {
            *value = REDACTED_HEADER_VALUE.to_string();
        }
    }
}

fn is_sensitive_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "authorization",
        "cookie",
        "token",
        "secret",
        "api-key",
        "apikey",
        "password",
        "session",
    ]
    .iter()
    .any(|marker| name.contains(marker))
}

fn response_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut captured = BTreeMap::<String, String>::new();
    for (key, value) in headers {
        let Ok(value) = value.to_str() else { continue };
        let key = key.as_str().to_ascii_lowercase();
        if key == "x-robots-tag" {
            captured
                .entry(key)
                .and_modify(|existing| {
                    existing.push_str(", ");
                    existing.push_str(value);
                })
                .or_insert_with(|| value.to_string());
        } else {
            captured.insert(key, value.to_string());
        }
    }
    redact_sensitive_headers(&mut captured);
    captured
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

#[cfg(test)]
mod tests {
    use super::{
        CrawlEgress, HttpFetcher, is_refused, origin_for_url, same_crawl_origin, same_site_host,
    };
    use crate::config::Config;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    /// Serves `respond(path)` as raw HTTP and records every request head, so a
    /// test can assert exactly what crossed the wire. Runs until dropped.
    struct RecordingServer {
        port: u16,
        requests: Arc<Mutex<Vec<String>>>,
        stop: Arc<Mutex<bool>>,
    }

    impl RecordingServer {
        fn start(respond: impl Fn(&str) -> String + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let port = listener.local_addr().unwrap().port();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(Mutex::new(false));
            let (recorded, stopped) = (requests.clone(), stop.clone());
            thread::spawn(move || {
                while !*stopped.lock().unwrap() {
                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
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
                    let _ = stream.flush();
                }
            });
            Self {
                port,
                requests,
                stop,
            }
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Drop for RecordingServer {
        fn drop(&mut self) {
            *self.stop.lock().unwrap() = true;
        }
    }

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn redirect(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    fn credentialed_config() -> Config {
        let mut config = Config::default();
        config
            .crawl_headers
            .insert("x-secret".to_string(), "header-secret".to_string());
        config.crawl_cookies.push(
            [("session".to_string(), "cookie-secret".to_string())]
                .into_iter()
                .collect(),
        );
        config
            .crawl_basic_auth
            .insert("username".to_string(), "user".to_string());
        config
            .crawl_basic_auth
            .insert("password".to_string(), "pass".to_string());
        config
    }

    fn fetcher(config: &Config, origin: &str, allow_private: bool) -> HttpFetcher {
        HttpFetcher::new(&config.runtime(), &CrawlEgress::new(origin, allow_private)).unwrap()
    }

    fn assert_has_credentials(request: &str) {
        assert!(request.contains("x-secret: header-secret"), "{request}");
        assert!(request.contains("cookie: "), "{request}");
        assert!(request.contains("authorization: basic"), "{request}");
    }

    fn assert_no_credentials(request: &str) {
        assert!(!request.contains("x-secret"), "{request}");
        assert!(!request.contains("cookie"), "{request}");
        assert!(!request.contains("authorization"), "{request}");
    }

    #[test]
    fn preserves_multiple_robots_headers() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append("x-robots-tag", "noindex".parse().unwrap());
        headers.append("x-robots-tag", "nofollow".parse().unwrap());
        let captured = super::response_headers(&headers);
        assert_eq!(captured.get("x-robots-tag").unwrap(), "noindex, nofollow");
    }

    #[test]
    fn redacts_sensitive_response_headers_at_capture() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append("set-cookie", "session=abc; Secure".parse().unwrap());
        headers.append("x-auth-token", "abc".parse().unwrap());
        headers.append("authorization", "Bearer abc".parse().unwrap());
        headers.append("content-type", "text/html".parse().unwrap());
        let captured = super::response_headers(&headers);
        assert_eq!(captured["set-cookie"], "[redacted]");
        assert_eq!(captured["x-auth-token"], "[redacted]");
        assert_eq!(captured["authorization"], "[redacted]");
        assert_eq!(captured["content-type"], "text/html");
    }

    #[test]
    fn compares_bare_and_www_hosts_as_same_site() {
        assert!(same_site_host("dilitrust.com", "www.dilitrust.com"));
        assert!(same_site_host("www.example.com:443", "example.com"));
        assert!(!same_site_host("docs.example.com", "example.com"));
    }

    #[test]
    fn crawl_origin_compares_scheme_and_port_as_well_as_host() {
        assert!(same_crawl_origin(
            "https://example.com/",
            "https://www.example.com:443/a"
        ));
        assert!(!same_crawl_origin(
            "https://example.com/",
            "http://example.com/"
        ));
        assert!(!same_crawl_origin(
            "http://example.com/",
            "http://example.com:81/"
        ));
        assert!(!same_crawl_origin("https://example.com/", "not a url"));
    }

    #[test]
    fn derives_origin_from_effective_url() {
        assert_eq!(
            origin_for_url("https://www.example.com/path/to/page?x=1"),
            "https://www.example.com/"
        );
    }

    #[test]
    fn fetches_headers_and_body_in_process() {
        let server = RecordingServer::start(|_| {
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nX-Test: ok\r\nContent-Length: 14\r\nConnection: close\r\n\r\n<h1>hello</h1>".to_string()
        });
        let origin = format!("http://127.0.0.1:{}", server.port);
        let fetched = fetcher(&Config::default(), &origin, true)
            .fetch(&origin)
            .unwrap();
        assert_eq!(fetched.status_code, Some(200));
        assert_eq!(
            fetched.headers.get("x-test").map(String::as_str),
            Some("ok")
        );
        assert_eq!(fetched.body.as_deref(), Some("<h1>hello</h1>"));
    }

    #[test]
    fn credentials_do_not_follow_a_redirect_to_another_host() {
        let other = RecordingServer::start(|_| ok("elsewhere"));
        let location = format!("http://localhost:{}/landing", other.port);
        let crawl = RecordingServer::start(move |_| redirect(&location));
        let origin = format!("http://127.0.0.1:{}", crawl.port);

        let fetched = fetcher(&credentialed_config(), &origin, true)
            .fetch(&origin)
            .unwrap();

        assert_eq!(fetched.body.as_deref(), Some("elsewhere"));
        assert_has_credentials(&crawl.requests()[0]);
        assert_eq!(other.requests().len(), 1);
        assert_no_credentials(&other.requests()[0]);
    }

    #[test]
    fn credentials_do_not_follow_a_redirect_to_another_port() {
        let other = RecordingServer::start(|_| ok("other port"));
        let location = format!("http://127.0.0.1:{}/", other.port);
        let crawl = RecordingServer::start(move |_| redirect(&location));
        let origin = format!("http://127.0.0.1:{}", crawl.port);

        fetcher(&credentialed_config(), &origin, true)
            .fetch(&origin)
            .unwrap();

        assert_has_credentials(&crawl.requests()[0]);
        assert_no_credentials(&other.requests()[0]);
    }

    #[test]
    fn credentials_follow_a_redirect_within_the_origin() {
        let crawl = RecordingServer::start(|path| match path {
            "/" => redirect("/final"),
            _ => ok("final"),
        });
        let origin = format!("http://127.0.0.1:{}", crawl.port);

        let fetched = fetcher(&credentialed_config(), &origin, true)
            .fetch(&origin)
            .unwrap();

        assert_eq!(fetched.body.as_deref(), Some("final"));
        let requests = crawl.requests();
        assert_eq!(requests.len(), 2);
        assert_has_credentials(&requests[0]);
        assert_has_credentials(&requests[1]);
    }

    /// `check_url` and the resolver run on every hop of the same loop, so
    /// refusing the first hop proves the gate a redirect hop goes through.
    #[test]
    fn strict_egress_refuses_private_targets_before_connecting() {
        let server = RecordingServer::start(|_| ok("private"));
        let strict = fetcher(&credentialed_config(), "https://example.com/", false);
        for url in [
            format!("http://127.0.0.1:{}/", server.port),
            format!("http://localhost:{}/", server.port),
        ] {
            let error = strict.fetch(&url).expect_err(&url);
            assert!(is_refused(&error), "{url}: {error:#}");
            assert!(error.to_string().contains("non-public"), "{error}");
            assert!(
                error.to_string().contains("allow_private_network"),
                "{error}"
            );
        }
        assert!(server.requests().is_empty());
    }

    #[test]
    fn fetch_errors_do_not_repeat_url_secrets() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let origin = format!("http://127.0.0.1:{port}");
        let error = fetcher(&Config::default(), &origin, true)
            .fetch(&format!(
                "http://user:hunter2@127.0.0.1:{port}/a?token=hunter2"
            ))
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(!message.contains("hunter2"), "{message}");
    }

    #[test]
    fn a_redirect_loop_is_refused() {
        let crawl = RecordingServer::start(|_| redirect("/again"));
        let origin = format!("http://127.0.0.1:{}", crawl.port);
        let error = fetcher(&Config::default(), &origin, true)
            .fetch(&origin)
            .unwrap_err();
        assert!(is_refused(&error), "{error:#}");
        assert_eq!(crawl.requests().len(), super::MAX_REDIRECTS + 1);
    }

    #[test]
    fn a_redirect_to_another_scheme_is_refused() {
        let crawl = RecordingServer::start(|_| redirect("file:///etc/passwd"));
        let origin = format!("http://127.0.0.1:{}", crawl.port);
        let error = fetcher(&Config::default(), &origin, true)
            .fetch(&origin)
            .unwrap_err();
        assert!(is_refused(&error), "{error:#}");
    }

    fn capped_fetcher(origin: &str, max_response_bytes: u64) -> HttpFetcher {
        let mut config = Config::default();
        config.crawl_max_response_bytes = max_response_bytes;
        fetcher(&config, origin, true)
    }

    #[test]
    fn oversized_declared_body_is_a_refusal() {
        let server = RecordingServer::start(|_| ok(&"x".repeat(1024)));
        let origin = format!("http://127.0.0.1:{}", server.port);
        let error = capped_fetcher(&origin, 100).fetch(&origin).unwrap_err();
        assert!(is_refused(&error), "{error:#}");
        assert!(
            error.to_string().contains("crawl_max_response_bytes"),
            "{error}"
        );
    }

    /// Without Content-Length only the streaming cap can stop the read, and it
    /// must do so long before the server finishes sending.
    #[test]
    fn oversized_undeclared_body_stops_reading_at_the_cap() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            // Endless body: this only ends when the client hangs up.
            let chunk = [b'x'; 8192];
            while stream.write_all(&chunk).is_ok() {}
        });
        let origin = format!("http://127.0.0.1:{port}");
        let started = Instant::now();
        let error = capped_fetcher(&origin, 64 * 1024)
            .fetch(&origin)
            .unwrap_err();
        assert!(is_refused(&error), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn body_at_the_cap_is_returned() {
        let server = RecordingServer::start(|_| ok(&"x".repeat(100)));
        let origin = format!("http://127.0.0.1:{}", server.port);
        let fetched = capped_fetcher(&origin, 100).fetch(&origin).unwrap();
        assert_eq!(fetched.body.map(|body| body.len()), Some(100));
    }

    #[test]
    fn decodes_the_declared_charset() {
        assert_eq!(
            super::decode_body(
                &[0x63, 0x61, 0x66, 0xe9],
                Some("text/html; Charset=\"iso-8859-1\"")
            ),
            "café"
        );
        assert_eq!(
            super::decode_body("café".as_bytes(), Some("text/html")),
            "café"
        );
    }
}
