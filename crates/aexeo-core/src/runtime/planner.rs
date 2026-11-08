use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet, VecDeque};

use super::graph::route_is_allowed;
use super::http::{host_for_url, normalize_base_url, origin_for_url, same_crawl_origin};
use super::robots::RobotsPolicy;
use crate::config::RuntimeConfig;
use crate::link_graph::redact_url;
use crate::site::{normalize_internal_href, route_from_urlish};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CrawlPlannerState {
    pub(crate) normalized_base: String,
    pub(crate) base_host: String,
    pub(crate) queue: Vec<String>,
    pub(crate) visited: Vec<String>,
    pub(crate) discovered_routes: Vec<String>,
    pub(crate) max_pages: usize,
    pub(crate) truncated: bool,
    #[serde(default)]
    pub(crate) robots_skipped_urls: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct CrawlPlanner {
    normalized_base: String,
    base_host: String,
    queue: VecDeque<String>,
    queued: HashSet<String>,
    visited: HashSet<String>,
    discovered_routes: HashSet<String>,
    max_pages: usize,
    truncated: bool,
    robots: RobotsPolicy,
    robots_skipped_urls: BTreeSet<String>,
}

impl CrawlPlanner {
    pub(crate) fn new(base_url: &str, max_pages: usize) -> Self {
        let normalized_base = normalize_base_url(base_url);
        Self {
            base_host: host_for_url(&normalized_base),
            queue: VecDeque::from([String::new()]),
            normalized_base,
            queued: HashSet::from([String::new()]),
            visited: HashSet::new(),
            discovered_routes: HashSet::from([String::new()]),
            max_pages,
            truncated: false,
            robots: RobotsPolicy::default(),
            robots_skipped_urls: BTreeSet::new(),
        }
    }

    pub(crate) fn normalized_base(&self) -> &str {
        &self.normalized_base
    }

    pub(crate) fn base_host(&self) -> &str {
        &self.base_host
    }

    pub(crate) fn discovered_route_count(&self) -> usize {
        self.discovered_routes.len()
    }

    pub(crate) fn visited_count(&self) -> usize {
        self.visited.len()
    }

    pub(crate) fn queued_count(&self) -> usize {
        self.queue.len()
    }

    pub(crate) fn truncated(&self) -> bool {
        self.truncated
    }

    pub(crate) fn checkpoint_state(&self) -> CrawlPlannerState {
        CrawlPlannerState {
            normalized_base: redact_url(&self.normalized_base),
            base_host: self.base_host.clone(),
            queue: self.queue.iter().cloned().collect(),
            visited: sorted_vec(&self.visited),
            discovered_routes: sorted_vec(&self.discovered_routes),
            max_pages: self.max_pages,
            truncated: self.truncated,
            robots_skipped_urls: self.robots_skipped_urls(),
        }
    }

    /// Restore a crawl. Where requests go comes only from `base_url`, which the
    /// current run supplies: a checkpoint is a file, and the persisted base must
    /// not be able to send this run's credentials elsewhere. Routes are relative
    /// to the origin, and redirects re-establish any alignment the crawl had.
    pub(crate) fn from_checkpoint(
        state: CrawlPlannerState,
        base_url: &str,
        max_pages: usize,
    ) -> Self {
        let queue_routes = state
            .queue
            .iter()
            .filter_map(|entry| canonicalize_route_entry(entry))
            .collect::<Vec<_>>();
        Self {
            normalized_base: origin_for_url(base_url),
            base_host: host_for_url(base_url),
            queue: VecDeque::from(queue_routes.clone()),
            queued: HashSet::from_iter(queue_routes),
            visited: state
                .visited
                .into_iter()
                .filter_map(|entry| canonicalize_route_entry(&entry))
                .collect(),
            discovered_routes: state
                .discovered_routes
                .into_iter()
                .filter_map(|entry| canonicalize_route_entry(&entry))
                .collect(),
            max_pages,
            truncated: false,
            robots: RobotsPolicy::default(),
            robots_skipped_urls: state.robots_skipped_urls.into_iter().collect(),
        }
    }

    pub(crate) fn set_robots_policy(&mut self, robots: RobotsPolicy) {
        self.robots = robots;
    }

    pub(crate) fn robots(&self) -> &RobotsPolicy {
        &self.robots
    }

    pub(crate) fn robots_skipped_urls(&self) -> Vec<String> {
        self.robots_skipped_urls.iter().cloned().collect()
    }

    pub(crate) fn robots_skipped_count(&self) -> usize {
        self.robots_skipped_urls.len()
    }

    /// Record `url` as skipped when robots.txt disallows it.
    pub(crate) fn skip_if_disallowed(&mut self, url: &str) -> bool {
        let disallowed = !self.robots.allows_url(url);
        if disallowed {
            self.robots_skipped_urls.insert(redact_url(url));
        }
        disallowed
    }

    pub(crate) fn seed_from_user_input(&mut self, seed: &str, runtime: &RuntimeConfig<'_>) {
        let Some(route) = route_from_urlish(seed).or_else(|| normalize_internal_href(seed)) else {
            return;
        };
        self.enqueue_route(route, runtime);
    }

    pub(crate) fn seed_from_sitemap_loc(&mut self, loc: &str, runtime: &RuntimeConfig<'_>) {
        let Some(route) = route_from_urlish(loc) else {
            return;
        };
        self.enqueue_route(route, runtime);
    }

    pub(crate) fn next_url(&mut self, runtime: &RuntimeConfig<'_>) -> Option<String> {
        // The limit is checked before a route is removed: a route that does not
        // fit stays queued, so a checkpoint can resume it.
        while let Some(route) = self.queue.front().cloned() {
            let url = self.url_for_route(&route);
            let skip = self.visited.contains(&route)
                || (!route.is_empty() && !route_is_allowed(&route, runtime))
                || self.skip_if_disallowed(&url);
            if !skip && self.visited.len() >= self.max_pages {
                self.truncated = true;
                return None;
            }
            self.queue.pop_front();
            self.queued.remove(&route);
            if !skip {
                self.visited.insert(route);
                return Some(url);
            }
        }
        None
    }

    pub(crate) fn discover_link_target(&mut self, target: &str, runtime: &RuntimeConfig<'_>) {
        let Some(route) = canonicalize_route_entry(target) else {
            return;
        };
        self.enqueue_route(route, runtime);
    }

    pub(crate) fn align_with_effective_url(&mut self, effective_url: &str) {
        // Only `www.` canonicalization rebases the crawl. A scheme or port
        // change must not: a redirect would otherwise move every later request
        // to an origin the user never named.
        let effective_host = host_for_url(effective_url);
        if effective_host.is_empty() || !same_crawl_origin(&self.normalized_base, effective_url) {
            return;
        }
        self.base_host = effective_host;
        self.normalized_base = origin_for_url(effective_url);
    }

    fn enqueue_route(&mut self, route: String, runtime: &RuntimeConfig<'_>) {
        if !route_is_allowed(&route, runtime) {
            return;
        }
        let url = self.url_for_route(&route);
        if host_for_url(&url) != self.base_host {
            return;
        }
        self.discovered_routes.insert(route.clone());
        if self.visited.contains(&route) || !self.queued.insert(route.clone()) {
            return;
        }
        self.queue.push_back(route);
    }

    fn url_for_route(&self, route: &str) -> String {
        if route.is_empty() {
            self.normalized_base.clone()
        } else {
            format!("{}{}", self.normalized_base, route)
        }
    }
}

fn canonicalize_route_entry(entry: &str) -> Option<String> {
    if entry.is_empty() {
        return Some(String::new());
    }
    if entry.contains("://") || entry.starts_with('/') {
        return route_from_urlish(entry).or_else(|| normalize_internal_href(entry));
    }
    Some(entry.trim_matches('/').to_string())
}

fn sorted_vec(values: &HashSet<String>) -> Vec<String> {
    let mut entries = values.iter().cloned().collect::<Vec<_>>();
    entries.sort();
    entries
}

#[cfg(test)]
mod tests {
    use super::{CrawlPlanner, CrawlPlannerState, RobotsPolicy};
    use crate::config::Config;

    #[test]
    fn planner_seeds_and_limits_routes() {
        let config = Config::default();
        let runtime = config.runtime();
        let mut planner = CrawlPlanner::new("https://example.com", 1);
        planner.seed_from_user_input("/docs", &runtime);

        assert_eq!(
            planner.next_url(&runtime).as_deref(),
            Some("https://example.com/")
        );
        assert!(planner.next_url(&runtime).is_none());
        assert!(planner.truncated());
        assert_eq!(planner.discovered_route_count(), 2);
    }

    #[test]
    fn a_route_beyond_the_limit_stays_queued_for_resume() {
        let config = Config::default();
        let runtime = config.runtime();
        let mut planner = CrawlPlanner::new("https://example.com", 1);
        planner.seed_from_user_input("/docs", &runtime);
        planner.next_url(&runtime);
        assert!(planner.next_url(&runtime).is_none());
        assert!(planner.truncated());
        assert_eq!(planner.checkpoint_state().queue, ["docs"]);

        let mut resumed =
            CrawlPlanner::from_checkpoint(planner.checkpoint_state(), "https://example.com", 2);
        assert_eq!(
            resumed.next_url(&runtime).as_deref(),
            Some("https://example.com/docs")
        );
    }

    #[test]
    fn exhausting_the_queue_exactly_at_the_limit_is_not_truncation() {
        let config = Config::default();
        let runtime = config.runtime();
        let mut planner = CrawlPlanner::new("https://example.com", 1);
        planner.seed_from_user_input("/", &runtime);
        planner.next_url(&runtime);
        assert!(planner.next_url(&runtime).is_none());
        assert!(!planner.truncated());
    }

    #[test]
    fn disallowed_routes_are_recorded_and_never_returned_including_after_resume() {
        let config = Config::default();
        let runtime = config.runtime();
        let robots = RobotsPolicy::parse("User-agent: *\nDisallow: /private\n", "aexeo");
        let mut planner = CrawlPlanner::new("https://example.com", 5);
        planner.set_robots_policy(robots.clone());
        planner.seed_from_user_input("/private", &runtime);
        planner.seed_from_user_input("/docs", &runtime);
        assert_eq!(
            planner.next_url(&runtime).as_deref(),
            Some("https://example.com/")
        );
        let mut resumed =
            CrawlPlanner::from_checkpoint(planner.checkpoint_state(), "https://example.com", 5);
        resumed.set_robots_policy(robots);
        assert_eq!(
            resumed.next_url(&runtime).as_deref(),
            Some("https://example.com/docs")
        );
        assert!(resumed.next_url(&runtime).is_none());
        assert_eq!(
            resumed.robots_skipped_urls(),
            ["https://example.com/private"]
        );
        assert!(!resumed.truncated());
    }

    #[test]
    fn planner_adopts_canonical_same_site_host() {
        let config = Config::default();
        let runtime = config.runtime();
        let mut planner = CrawlPlanner::new("https://example.com", 10);
        planner.align_with_effective_url("https://www.example.com/features/test");
        planner.discover_link_target("docs", &runtime);

        assert_eq!(planner.base_host(), "www.example.com");
        assert_eq!(planner.normalized_base(), "https://www.example.com/");
        assert_eq!(
            planner.next_url(&runtime).as_deref(),
            Some("https://www.example.com/")
        );
        assert_eq!(
            planner.next_url(&runtime).as_deref(),
            Some("https://www.example.com/docs")
        );
    }

    #[test]
    fn planner_does_not_rebase_on_scheme_or_port_change() {
        for effective in [
            "http://example.com/",
            "http://example.com:81/",
            "https://example.com:8443/",
            "http://www.example.com/",
        ] {
            let mut planner = CrawlPlanner::new("https://example.com", 10);
            planner.align_with_effective_url(effective);
            assert_eq!(
                planner.normalized_base(),
                "https://example.com/",
                "{effective}"
            );
            assert_eq!(planner.base_host(), "example.com", "{effective}");
        }
    }

    #[test]
    fn planner_round_trips_checkpoint_state() {
        let config = Config::default();
        let runtime = config.runtime();
        let mut planner = CrawlPlanner::new("https://example.com", 10);
        planner.seed_from_user_input("/docs", &runtime);
        let _ = planner.next_url(&runtime);

        let state = planner.checkpoint_state();
        let restored = CrawlPlanner::from_checkpoint(
            CrawlPlannerState {
                max_pages: 20,
                ..state
            },
            "https://example.com",
            20,
        );
        assert_eq!(restored.base_host(), "example.com");
        assert_eq!(restored.discovered_route_count(), 2);
        assert_eq!(restored.visited_count(), 1);
    }

    #[test]
    fn a_resumed_planner_targets_the_given_url_not_the_persisted_one() {
        let config = Config::default();
        let runtime = config.runtime();
        let mut planner = CrawlPlanner::new("https://www.example.com/start", 10);
        planner.align_with_effective_url("https://www.example.com/start");
        planner.seed_from_user_input("/docs", &runtime);
        let tampered = CrawlPlannerState {
            normalized_base: "http://evil.test:8080/".to_string(),
            base_host: "evil.test:8080".to_string(),
            ..planner.checkpoint_state()
        };
        let mut resumed =
            CrawlPlanner::from_checkpoint(tampered, "https://www.example.com/start", 10);
        assert_eq!(resumed.normalized_base(), "https://www.example.com/");
        assert_eq!(resumed.base_host(), "www.example.com");
        assert_eq!(
            resumed.next_url(&runtime).as_deref(),
            Some("https://www.example.com/")
        );
    }

    #[test]
    fn a_checkpoint_does_not_persist_url_credentials() {
        let planner = CrawlPlanner::new("https://user:secret@example.com", 10);
        assert_eq!(
            planner.checkpoint_state().normalized_base,
            "https://example.com/"
        );
    }

    #[test]
    fn planner_deduplicates_discovered_routes_in_queue() {
        let config = Config::default();
        let runtime = config.runtime();
        let mut planner = CrawlPlanner::new("https://example.com", 10);
        planner.discover_link_target("docs", &runtime);
        planner.discover_link_target("/docs", &runtime);
        planner.discover_link_target("https://example.com/docs", &runtime);

        assert_eq!(planner.discovered_route_count(), 2);
        assert_eq!(planner.queued_count(), 2);
        assert_eq!(
            planner.next_url(&runtime).as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(
            planner.next_url(&runtime).as_deref(),
            Some("https://example.com/docs")
        );
        assert!(planner.next_url(&runtime).is_none());
    }
}
