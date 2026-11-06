//! The crawler's network trust boundary.
//!
//! A crawl fetches URLs chosen by the site it is auditing: redirects, sitemap
//! `<loc>` entries, manifest paths. `CrawlEgress` decides, per hop, whether a
//! URL may be contacted and whether it may receive the configured credentials.
//!
//! Private-network blocking is enforced in the DNS resolver handed to the HTTP
//! client, so the addresses that were checked are the addresses connected to,
//! and a DNS rebind between check and connect has no window. IP-literal URLs
//! never reach a resolver and are checked per hop in `check_url`.
//!
//! Residual risks: a configured HTTP proxy resolves names itself, so only
//! IP-literal URLs are checked behind a proxy; and a target whose DNS answers
//! are non-public at crawl start is treated as local preview for that crawl.

use aexeo_contracts::{Finding, FindingScope};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use url::{Host, Url};

use super::graph::response_report_path;
use super::http::same_crawl_origin;
use crate::link_graph::redact_url;
use crate::site::route_from_urlish;

/// A fetch the crawl policy refused, as opposed to a network failure. Refusals
/// are deterministic, so they are reported as findings and never retried.
#[derive(Debug, Clone)]
pub(crate) struct FetchRefused(pub(crate) String);

impl fmt::Display for FetchRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for FetchRefused {}

pub(crate) fn is_refused(error: &anyhow::Error) -> bool {
    error.downcast_ref::<FetchRefused>().is_some()
}

#[derive(Debug, Clone)]
pub(crate) struct CrawlEgress {
    origin: String,
    allow_private: bool,
}

impl CrawlEgress {
    /// Fix the crawl origin and network scope from the crawl target. Resolves
    /// the target once: local preview keeps working because a target that is
    /// itself non-public opts the crawl into private-network access.
    pub(crate) fn for_target(base_url: &str, allow_private_network: bool) -> Self {
        let allow_private = allow_private_network
            || Url::parse(base_url).is_ok_and(|url| target_is_non_public(&url));
        Self::new(base_url, allow_private)
    }

    pub(crate) fn new(origin: &str, allow_private: bool) -> Self {
        Self {
            origin: origin.to_string(),
            allow_private,
        }
    }

    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    pub(crate) fn allow_private(&self) -> bool {
        self.allow_private
    }

    /// Credentials go only to the crawl origin: same scheme, host and port,
    /// with `www.` equivalence. An https to http downgrade or a port change
    /// therefore never carries them.
    pub(crate) fn allows_credentials(&self, url: &Url) -> bool {
        same_crawl_origin(&self.origin, url.as_str())
    }

    pub(crate) fn check_url(&self, url: &Url) -> Result<(), FetchRefused> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(FetchRefused(format!(
                "refused to fetch {}: scheme `{}` is not allowed",
                redact_url(url.as_str()),
                url.scheme()
            )));
        }
        let literal = match url.host() {
            Some(Host::Ipv4(ip)) => Some(IpAddr::V4(ip)),
            Some(Host::Ipv6(ip)) => Some(IpAddr::V6(ip)),
            _ => None,
        };
        if let Some(ip) = literal
            && !self.allow_private
            && !is_public_ip(ip)
        {
            return Err(FetchRefused(non_public_message(&redact_url(url.as_str()))));
        }
        Ok(())
    }

    pub(crate) fn resolver(&self) -> Arc<GuardedResolver> {
        Arc::new(GuardedResolver {
            allow_private: self.allow_private,
        })
    }
}

fn non_public_message(target: &str) -> String {
    format!(
        "refused to fetch {target}: it resolves to a non-public network address and the crawl target does not; set `[runtime] allow_private_network = true` if this target is intended"
    )
}

/// Finding for a fetch the policy refused; other errors yield `None`.
pub(crate) fn refused_finding(url: &str, error: &anyhow::Error) -> Option<Finding> {
    error
        .downcast_ref::<FetchRefused>()
        .map(|refused| crawl_warning(url, refused.to_string()))
}

pub(crate) fn crawl_warning(url: &str, message: String) -> Finding {
    Finding {
        rule_id: "CRW001".to_string(),
        path: response_report_path(&route_from_urlish(url).unwrap_or_default()),
        message,
        line: 1,
        column: 1,
        severity: "warning".to_string(),
        suggestion: None,
        scope: FindingScope::Page,
    }
}

/// Resolves through the OS, then drops non-public answers unless the crawl
/// allows them. Only surviving addresses reach the connector.
#[derive(Debug)]
pub(crate) struct GuardedResolver {
    allow_private: bool,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allow_private = self.allow_private;
        Box::pin(async move {
            let host = name.as_str().to_string();
            let lookup_host = host.clone();
            let resolved = tokio::task::spawn_blocking(move || {
                (lookup_host.as_str(), 0)
                    .to_socket_addrs()
                    .map(Vec::from_iter)
            })
            .await??;
            let permitted = permitted_addresses(resolved, allow_private);
            if permitted.is_empty() {
                let refused = FetchRefused(non_public_message(&host));
                return Err(Box::new(refused) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(permitted.into_iter()) as Addrs)
        })
    }
}

fn permitted_addresses(resolved: Vec<SocketAddr>, allow_private: bool) -> Vec<SocketAddr> {
    if allow_private {
        return resolved;
    }
    resolved
        .into_iter()
        .filter(|address| is_public_ip(address.ip()))
        .collect()
}

fn target_is_non_public(url: &Url) -> bool {
    let addresses: Vec<IpAddr> = match url.host() {
        Some(Host::Ipv4(ip)) => vec![IpAddr::V4(ip)],
        Some(Host::Ipv6(ip)) => vec![IpAddr::V6(ip)],
        Some(Host::Domain(domain)) => (domain, 0)
            .to_socket_addrs()
            .map(|addresses| addresses.map(|address| address.ip()).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    !addresses.is_empty() && addresses.into_iter().all(|ip| !is_public_ip(ip))
}

pub(crate) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    let non_public = a == 0                                   // "this network"
        || a == 10                                            // RFC 1918
        || a == 127                                           // loopback
        || (a == 100 && (64..128).contains(&b))               // CGNAT
        || (a == 169 && b == 254)                             // link-local, cloud metadata
        || (a == 172 && (16..32).contains(&b))                // RFC 1918
        || (a == 192 && b == 0 && c == 0)                     // IETF protocol assignments
        || (a == 192 && b == 0 && c == 2)                     // documentation
        || (a == 192 && b == 168)                             // RFC 1918
        || (a == 198 && (b == 18 || b == 19))                 // benchmarking
        || (a == 198 && b == 51 && c == 100)                  // documentation
        || (a == 203 && b == 0 && c == 113)                   // documentation
        || a >= 224; // multicast, reserved, broadcast
    !non_public
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    // Mapped (::ffff:a.b.c.d) and compatible (::a.b.c.d, including :: and ::1)
    // forms reach IPv4 space, so they are judged as IPv4.
    if let Some(embedded) = ip.to_ipv4() {
        return is_public_v4(embedded);
    }
    let segments = ip.segments();
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let [_, _, _, _, _, _, high, low] = segments;
        return is_public_v4(Ipv4Addr::from((u32::from(high) << 16) | u32::from(low)));
    }
    let non_public = (segments[0] & 0xfe00) == 0xfc00 // unique local
        || (segments[0] & 0xffc0) == 0xfe80           // link-local
        || (segments[0] & 0xffc0) == 0xfec0           // deprecated site-local
        || (segments[0] & 0xff00) == 0xff00           // multicast
        || (segments[0] == 0x2001 && segments[1] == 0x0db8); // documentation
    !non_public
}

#[cfg(test)]
mod tests {
    use super::{CrawlEgress, is_public_ip, permitted_addresses};
    use std::net::{IpAddr, SocketAddr};
    use url::Url;

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[test]
    fn classifies_non_public_addresses() {
        for value in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::a9fe:a9fe",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
        ] {
            assert!(!is_public_ip(ip(value)), "{value} must not be public");
        }
    }

    #[test]
    fn classifies_public_addresses() {
        for value in [
            "1.1.1.1",
            "8.8.8.8",
            "172.32.0.1",
            "100.128.0.1",
            "192.169.0.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
        ] {
            assert!(is_public_ip(ip(value)), "{value} must be public");
        }
    }

    #[test]
    fn credentials_require_scheme_host_and_port_with_www_equivalence() {
        let egress = CrawlEgress::new("https://example.com/", false);
        let allowed = |url: &str| egress.allows_credentials(&Url::parse(url).unwrap());
        assert!(allowed("https://example.com/page"));
        assert!(allowed("https://www.example.com/page"));
        assert!(allowed("https://EXAMPLE.com:443/page"));
        assert!(!allowed("http://example.com/page"), "downgrade");
        assert!(!allowed("https://example.com:8443/page"), "port change");
        assert!(
            !allowed("http://www.example.com/page"),
            "www with downgrade"
        );
        assert!(!allowed("https://docs.example.com/page"), "subdomain");
        assert!(!allowed("https://example.com.evil.test/page"), "suffix");
    }

    #[test]
    fn check_url_refuses_private_literals_unless_allowed() {
        let strict = CrawlEgress::new("https://example.com/", false);
        let local = CrawlEgress::new("http://127.0.0.1/", true);
        for url in [
            "http://127.0.0.1/",
            "http://2130706433/",
            "http://[::ffff:7f00:1]/",
            "http://169.254.169.254/latest/meta-data/",
        ] {
            let parsed = Url::parse(url).unwrap();
            let refusal = strict.check_url(&parsed).expect_err(url);
            assert!(refusal.to_string().contains("non-public"), "{refusal}");
            assert!(local.check_url(&parsed).is_ok(), "{url}");
        }
        assert!(
            strict
                .check_url(&Url::parse("https://1.1.1.1/").unwrap())
                .is_ok()
        );
    }

    #[test]
    fn refusals_do_not_repeat_url_secrets() {
        let strict = CrawlEgress::new("https://example.com/", false);
        let url = Url::parse("http://user:hunter2@127.0.0.1/a?token=hunter2#hunter2").unwrap();
        let refusal = strict.check_url(&url).unwrap_err().to_string();
        assert!(!refusal.contains("hunter2"), "{refusal}");
        assert!(refusal.contains("http://127.0.0.1/a"), "{refusal}");
        let finding = super::crawl_warning(url.as_str(), refusal);
        assert!(!finding.path.contains("hunter2"), "{}", finding.path);
    }

    #[test]
    fn check_url_refuses_other_schemes() {
        let egress = CrawlEgress::new("http://127.0.0.1/", true);
        assert!(
            egress
                .check_url(&Url::parse("file:///etc/passwd").unwrap())
                .is_err()
        );
        assert!(
            egress
                .check_url(&Url::parse("ftp://example.com/").unwrap())
                .is_err()
        );
    }

    #[test]
    fn resolver_filter_keeps_only_public_answers() {
        let answers: Vec<SocketAddr> = vec![
            "127.0.0.1:0".parse().unwrap(),
            "93.184.216.34:0".parse().unwrap(),
            "[fd00::1]:0".parse().unwrap(),
        ];
        assert_eq!(
            permitted_addresses(answers.clone(), false),
            vec!["93.184.216.34:0".parse::<SocketAddr>().unwrap()]
        );
        assert_eq!(permitted_addresses(answers.clone(), true), answers);
    }
}
