use std::collections::{BTreeSet, VecDeque};

use super::graph::route_is_allowed;
use super::http::{host_for_url, normalize_base_url};
use crate::config::RuntimeConfig;
use crate::site::{normalize_internal_href, route_from_urlish};

#[derive(Debug)]
pub(crate) struct CrawlPlanner {
    normalized_base: String,
    base_host: String,
    queue: VecDeque<String>,
    visited: BTreeSet<String>,
    discovered_routes: BTreeSet<String>,
    max_pages: usize,
    truncated: bool,
}

impl CrawlPlanner {
    pub(crate) fn new(base_url: &str, max_pages: usize) -> Self {
        let normalized_base = normalize_base_url(base_url);
        Self {
            base_host: host_for_url(&normalized_base),
            queue: VecDeque::from([normalized_base.clone()]),
            normalized_base,
            visited: BTreeSet::new(),
            discovered_routes: BTreeSet::from([String::new()]),
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

    pub(crate) fn truncated(&self) -> bool {
        self.truncated
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
        while let Some(current) = self.queue.pop_front() {
            if self.visited.len() >= self.max_pages {
                self.truncated = true;
                return None;
            }
            if self.visited.contains(&current) {
                continue;
            }
            self.visited.insert(current.clone());
            let current_route = route_from_urlish(&current).unwrap_or_default();
            if !current_route.is_empty() && !route_is_allowed(&current_route, runtime) {
                continue;
            }
            return Some(current);
        }
        None
    }

    pub(crate) fn discover_link_target(&mut self, target: &str, runtime: &RuntimeConfig<'_>) {
        self.enqueue_route(target.to_string(), runtime);
    }

    fn enqueue_route(&mut self, route: String, runtime: &RuntimeConfig<'_>) {
        if !route_is_allowed(&route, runtime) {
            return;
        }
        let url = if route.is_empty() {
            self.normalized_base.clone()
        } else {
            format!("{}{}", self.normalized_base, route)
        };
        if host_for_url(&url) != self.base_host {
            return;
        }
        self.discovered_routes.insert(route);
        self.queue.push_back(url);
    }
}

#[cfg(test)]
mod tests {
    use super::CrawlPlanner;
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
}
