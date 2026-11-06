//! robots.txt evaluation for the crawler, following RFC 9309.
//!
//! The rule modules in `robots_rules` lint a site's robots.txt; this module
//! decides what the crawler itself may request.
use std::collections::BTreeMap;

use anyhow::Result;
use url::Url;

use super::fetcher::crawl_user_agent;
use super::http::{FetchResult, fetch_with_http};
use crate::config::RuntimeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rule {
    pattern: String,
    allow: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum RobotsPolicy {
    #[default]
    AllowAll,
    DenyAll,
    Rules(Vec<Rule>),
}

impl RobotsPolicy {
    /// Parse robots.txt for one product token. The groups naming the token apply;
    /// without any, the `*` groups do. Same-agent groups are merged.
    pub(crate) fn parse(text: &str, product_token: &str) -> Self {
        let mut named = Vec::new();
        let mut wildcard = Vec::new();
        let mut agents = Vec::<String>::new();
        let mut in_rules = false;
        let mut has_named_group = false;
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or_default();
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            let allow = match key.trim().to_ascii_lowercase().as_str() {
                "user-agent" => {
                    if in_rules {
                        agents.clear();
                        in_rules = false;
                    }
                    has_named_group |= names_product(value, product_token);
                    agents.push(value.to_string());
                    continue;
                }
                "allow" => true,
                "disallow" => false,
                _ => continue,
            };
            in_rules = true;
            if value.is_empty() {
                continue;
            }
            let rule = Rule {
                pattern: encode_non_ascii(value),
                allow,
            };
            if agents
                .iter()
                .any(|agent| names_product(agent, product_token))
            {
                named.push(rule);
            } else if agents.iter().any(|agent| agent == "*") {
                wildcard.push(rule);
            }
        }
        // A named group with only an empty `Disallow:` still overrides `*`.
        Self::Rules(if has_named_group { named } else { wildcard })
    }

    fn allows_path(&self, path: &str) -> bool {
        let rules = match self {
            Self::AllowAll => return true,
            Self::DenyAll => return path == "/robots.txt",
            Self::Rules(rules) => rules,
        };
        if path == "/robots.txt" {
            return true;
        }
        rules
            .iter()
            .filter(|rule| pattern_matches(&rule.pattern, path))
            .max_by_key(|rule| (rule.pattern.len(), rule.allow))
            .is_none_or(|rule| rule.allow)
    }

    /// Whether the crawler may request `url`. Routes do not retain a trailing
    /// slash and a server may redirect to either form, so both must be allowed:
    /// `Disallow: /private/` also keeps the crawler away from `/private`.
    pub(crate) fn allows_url(&self, url: &str) -> bool {
        let Ok(parsed) = Url::parse(url) else {
            return !matches!(self, Self::DenyAll);
        };
        let path = parsed.path();
        let bare = path.trim_end_matches('/');
        self.allows_path(path)
            && self.allows_path(&format!("{bare}/"))
            && (bare.is_empty() || self.allows_path(bare))
    }
}

fn names_product(agent: &str, product_token: &str) -> bool {
    let name = agent.split(['/', ' ']).next().unwrap_or_default();
    !name.is_empty() && name.eq_ignore_ascii_case(product_token)
}

/// URL paths are percent-encoded, so a pattern must be compared the same way.
fn encode_non_ascii(pattern: &str) -> String {
    let mut encoded = String::with_capacity(pattern.len());
    for ch in pattern.chars() {
        if ch.is_ascii() {
            encoded.push(ch);
        } else {
            let mut buffer = [0_u8; 4];
            for byte in ch.encode_utf8(&mut buffer).bytes() {
                encoded.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    encoded
}

/// Prefix match where `*` spans any run and a trailing `$` anchors the end.
/// Tracks reachable positions rather than backtracking, so a hostile pattern
/// such as `/*a*a*a*a*b` costs at most pattern length times path length.
fn pattern_matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(rest) => (rest, true),
        None => (pattern, false),
    };
    let path = path.as_bytes();
    let mut reachable = vec![false; path.len() + 1];
    reachable[0] = true;
    for &token in pattern.as_bytes() {
        let mut next = vec![false; path.len() + 1];
        if token == b'*' {
            let mut seen = false;
            for index in 0..=path.len() {
                seen |= reachable[index];
                next[index] = seen;
            }
        } else {
            for index in 0..path.len() {
                next[index + 1] = reachable[index] && path[index] == token;
            }
        }
        reachable = next;
    }
    if anchored {
        reachable[path.len()]
    } else {
        reachable.contains(&true)
    }
}

/// robots.txt as observed for an origin.
pub(crate) struct LoadedRobots {
    pub(crate) policy: RobotsPolicy,
    /// The body, kept for the rules that lint robots.txt.
    pub(crate) text: Option<String>,
    /// Why the file could not be read, when that restricts the crawl.
    pub(crate) unreachable: Option<String>,
}

pub(crate) fn load_robots(origin: &str, runtime: &RuntimeConfig<'_>) -> LoadedRobots {
    let result = fetch_with_http(
        &format!("{origin}robots.txt"),
        runtime.crawl_headers,
        runtime.crawl_cookies,
        runtime.crawl_basic_auth,
    );
    interpret_response(result, runtime.crawl_headers)
}

/// RFC 9309 section 2.3.1: a 4xx means there are no restrictions; a server error
/// or network failure means the rules are unknown, so everything is disallowed.
/// HTTP 429 asks the client to back off, so it counts as a server error here.
fn interpret_response(
    result: Result<FetchResult>,
    headers: &BTreeMap<String, String>,
) -> LoadedRobots {
    let fetched = match result {
        Ok(fetched) => fetched,
        Err(error) => {
            return LoadedRobots {
                policy: RobotsPolicy::DenyAll,
                text: None,
                unreachable: Some(format!("{error:#}")),
            };
        }
    };
    match (fetched.status_code, fetched.body) {
        (Some(200..=299), Some(body)) => {
            let agent = crawl_user_agent(headers);
            let token = agent.split(['/', ' ']).next().unwrap_or_default();
            LoadedRobots {
                policy: RobotsPolicy::parse(&body, token),
                text: Some(body),
                unreachable: None,
            }
        }
        (Some(status), _) if (400..500).contains(&status) && status != 429 => LoadedRobots {
            policy: RobotsPolicy::AllowAll,
            text: None,
            unreachable: None,
        },
        (status, _) => LoadedRobots {
            policy: RobotsPolicy::DenyAll,
            text: None,
            unreachable: Some(match status {
                Some(status) => format!("HTTP {status}"),
                None => "no HTTP status".to_string(),
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{RobotsPolicy, interpret_response};
    use crate::runtime::http::FetchResult;
    use std::collections::BTreeMap;

    fn policy(text: &str) -> RobotsPolicy {
        RobotsPolicy::parse(text, "aexeo")
    }

    fn allowed(text: &str, path: &str) -> bool {
        policy(text).allows_url(&format!("https://example.org{path}"))
    }

    #[test]
    fn a_named_group_replaces_the_wildcard_group() {
        let text = "User-agent: *\nDisallow: /all\n\nUser-agent: AEXEO/1.0\nDisallow: /named\n";
        assert!(!allowed(text, "/named"));
        assert!(allowed(text, "/all"));
        let other = "User-agent: *\nDisallow: /all\n\nUser-agent: other\nDisallow: /o\n";
        assert!(!allowed(other, "/all"));
        assert!(allowed(other, "/o"));
    }

    #[test]
    fn a_named_group_with_an_empty_disallow_allows_everything() {
        let text = "User-agent: *\nDisallow: /\n\nUser-agent: aexeo\nDisallow:\n";
        assert!(allowed(text, "/page"));
    }

    #[test]
    fn consecutive_user_agent_lines_share_their_rules_and_groups_merge() {
        let text =
            "User-agent: x\nUser-agent: aexeo\nDisallow: /a\nUser-agent: aexeo\nDisallow: /b\n";
        assert!(!allowed(text, "/a"));
        assert!(!allowed(text, "/b"));
        assert!(allowed(text, "/c"));
    }

    #[test]
    fn the_longest_match_wins_and_allow_wins_ties() {
        let text =
            "User-agent: *\nDisallow: /docs\nAllow: /docs/public\nDisallow: /tie\nAllow: /tie\n";
        assert!(!allowed(text, "/docs/secret"));
        assert!(allowed(text, "/docs/public/page"));
        assert!(allowed(text, "/tie"));
    }

    #[test]
    fn wildcards_anchors_comments_and_encoding_are_honoured() {
        let text =
            "User-agent: *\nDisallow: /*.pdf$ # documents\nDisallow: /a*/z\nDisallow: /café\n";
        assert!(!allowed(text, "/files/x.pdf"));
        assert!(allowed(text, "/files/x.pdf/view"));
        assert!(!allowed(text, "/abc/z"));
        assert!(allowed(text, "/abc/y"));
        assert!(!allowed(text, "/caf%C3%A9"));
    }

    #[test]
    fn a_hostile_pattern_does_not_blow_up() {
        let text = format!("User-agent: *\nDisallow: /{}b\n", "*a".repeat(200));
        assert!(allowed(&text, &format!("/{}", "a".repeat(2000))));
    }

    #[test]
    fn a_directory_rule_also_covers_the_route_without_its_slash() {
        let text = "User-agent: *\nDisallow: /private/\n";
        assert!(!allowed(text, "/private"));
        assert!(!allowed(text, "/private/x"));
        assert!(allowed(text, "/privateer"));
    }

    #[test]
    fn disallow_all_blocks_the_root_but_never_robots_txt() {
        let text = "User-agent: *\nDisallow: /\n";
        assert!(!allowed(text, "/"));
        assert!(allowed(text, "/robots.txt"));
    }

    fn response(status: u16, body: Option<&str>) -> anyhow::Result<FetchResult> {
        Ok(FetchResult {
            status_code: Some(status),
            content_type: None,
            body: body.map(str::to_string),
            headers: BTreeMap::new(),
            effective_url: String::new(),
            effective_url_observed: true,
            headers_observed: true,
        })
    }

    #[test]
    fn response_status_selects_the_policy_as_rfc_9309_requires() {
        let headers = BTreeMap::new();
        let check = |result| interpret_response(result, &headers);

        let ok = check(response(200, Some("User-agent: *\nDisallow: /x\n")));
        assert!(matches!(ok.policy, RobotsPolicy::Rules(_)));
        assert!(ok.text.is_some() && ok.unreachable.is_none());

        for status in [401, 403, 404, 410] {
            let missing = check(response(status, Some("<html>")));
            assert_eq!(missing.policy, RobotsPolicy::AllowAll, "{status}");
            assert!(missing.text.is_none() && missing.unreachable.is_none());
        }
        for status in [429, 500, 503] {
            let down = check(response(status, None));
            assert_eq!(down.policy, RobotsPolicy::DenyAll, "{status}");
            assert!(down.unreachable.is_some());
        }
        let failed = check(Err(anyhow::anyhow!("timed out")));
        assert_eq!(failed.policy, RobotsPolicy::DenyAll);
        assert!(failed.unreachable.unwrap().contains("timed out"));
    }
}
