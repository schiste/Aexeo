use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

use super::graph::route_is_allowed;
use super::http::{host_for_url, normalize_base_url, origin_for_url, same_site_host};
use crate::config::RuntimeConfig;
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
            normalized_base: self.normalized_base.clone(),
            base_host: self.base_host.clone(),
            queue: self.queue.iter().cloned().collect(),
            visited: sorted_vec(&self.visited),
            discovered_routes: sorted_vec(&self.discovered_routes),
            max_pages: self.max_pages,
            truncated: self.truncated,
        }
    }

    pub(crate) fn from_checkpoint(state: CrawlPlannerState, max_pages: usize) -> Self {
        let queue_routes = state
            .queue
            .iter()
            .filter_map(|entry| canonicalize_route_entry(entry))
            .collect::<Vec<_>>();
        Self {
            normalized_base: state.normalized_base,
            base_host: state.base_host,
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
        }
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
        while let Some(route) = self.queue.pop_front() {
            self.queued.remove(&route);
            if self.visited.len() >= self.max_pages {
                self.truncated = true;
                return None;
            }
            if self.visited.contains(&route) {
                continue;
            }
            if !route.is_empty() && !route_is_allowed(&route, runtime) {
                continue;
            }
            self.visited.insert(route.clone());
            return Some(self.url_for_route(&route));
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
        let effective_host = host_for_url(effective_url);
        if effective_host.is_empty() || !same_site_host(&self.base_host, &effective_host) {
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
    use super::{CrawlPlanner, CrawlPlannerState};
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
            20,
        );
        assert_eq!(restored.base_host(), "example.com");
        assert_eq!(restored.discovered_route_count(), 2);
        assert_eq!(restored.visited_count(), 1);
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
