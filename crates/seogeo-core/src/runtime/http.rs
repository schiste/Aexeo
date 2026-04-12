use anyhow::{Context, Result};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use std::collections::BTreeMap;
use std::time::Duration;
use url::Url;

#[derive(Debug, Clone)]
pub(crate) struct FetchResult {
    pub(crate) status_code: Option<u16>,
    pub(crate) content_type: Option<String>,
    pub(crate) body: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) effective_url: String,
}

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

fn build_client(headers: &BTreeMap<String, String>) -> Result<Client> {
    Ok(Client::builder()
        .default_headers(headers_from_map(headers)?)
        .redirect(Policy::limited(10))
        .timeout(Duration::from_secs(30))
        .build()?)
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

fn response_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(key, value)| {
            Some((
                key.as_str().to_ascii_lowercase(),
                value.to_str().ok()?.to_string(),
            ))
        })
        .collect()
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

pub(crate) fn fetch_with_http(
    url: &str,
    headers: &BTreeMap<String, String>,
    cookies: &[BTreeMap<String, String>],
    basic_auth: &BTreeMap<String, String>,
) -> Result<FetchResult> {
    let client = build_client(headers)?;
    let response = apply_runtime_credentials(client.get(url), cookies, basic_auth)
        .send()
        .with_context(|| format!("failed to fetch URL: {url}"))?;
    let status_code = Some(response.status().as_u16());
    let effective_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let headers = response_headers(response.headers());
    let body = response.text().ok();
    Ok(FetchResult {
        status_code,
        content_type,
        body,
        headers,
        effective_url,
    })
}

#[cfg(test)]
mod tests {
    use super::{fetch_with_http, origin_for_url, same_site_host};
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn compares_bare_and_www_hosts_as_same_site() {
        assert!(same_site_host("dilitrust.com", "www.dilitrust.com"));
        assert!(same_site_host("www.example.com:443", "example.com"));
        assert!(!same_site_host("docs.example.com", "example.com"));
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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nX-Test: ok\r\nContent-Length: 14\r\nConnection: close\r\n\r\n<h1>hello</h1>",
                )
                .unwrap();
            stream.flush().unwrap();
        });

        let fetched = fetch_with_http(
            &format!("http://{address}"),
            &BTreeMap::new(),
            &[],
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(fetched.status_code, Some(200));
        assert_eq!(
            fetched.headers.get("x-test").map(String::as_str),
            Some("ok")
        );
        assert_eq!(fetched.body.as_deref(), Some("<h1>hello</h1>"));
        handle.join().unwrap();
    }
}
