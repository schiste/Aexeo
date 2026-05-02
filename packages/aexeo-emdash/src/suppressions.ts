// Editor-workflow suppressions for plugin findings.
//
// Suppressions are a host policy, not an engine concern. The CLI deliberately
// stays strict — its findings reflect the canonical audit. The plugin's
// suppressions live one level up: they let editors silence specific routes or
// rules that are intentional violations the host has decided not to fix
// (legal pages with non-standard SEO; localized routes where a rule doesn't
// apply; rule-X-is-known-noise-here-while-we-rework).
//
// Filtering happens BEFORE findings are persisted to KV. Suppressed findings
// never reach the dashboard, the /findings page, or the per-document panel.
// This is intentional — if a suppression were applied at render time only,
// editors would see noise during refresh and have to ignore it manually.

import type { Finding } from "./types.js";

/// Single suppression rule. At least one of `routePattern` or `ruleIds` must
/// be set; `{}` is rejected at config-load time as a likely editor mistake.
///
/// Semantics:
///   - `routePattern` absent → matches any route
///   - `ruleIds` absent     → matches any rule
///   - both absent          → invalid (rejected)
///
/// `routePattern` uses glob syntax:
///   - `*`  matches any chars except `/`  (e.g. `/blog/*` matches `/blog/foo`
///                                          but not `/blog/foo/bar`)
///   - `**` matches any chars including `/`  (`/blog/**` matches both)
///   - `?`  matches exactly one char except `/`
///
/// Patterns are anchored: `/about` does not match `/about/team`. Use
/// `/about/**` if you want a subtree.
export interface Suppression {
  routePattern?: string;
  ruleIds?: readonly string[];
}

/// Pre-compiled filter. `apply` returns the input findings minus any matching
/// the configured suppressions. Empty input or empty rules → identity.
export interface SuppressionFilter {
  apply(route: string, findings: readonly Finding[]): Finding[];
}

const NOOP_FILTER: SuppressionFilter = {
  apply: (_route, findings) => [...findings],
};

/// Validates and compiles suppressions to a fast filter. Throws when a rule
/// has neither `routePattern` nor `ruleIds` — that's a kill-switch for all
/// findings everywhere and almost certainly an editor mistake.
export function compileSuppressions(
  rules: readonly Suppression[] | undefined,
): SuppressionFilter {
  if (rules === undefined || rules.length === 0) {
    return NOOP_FILTER;
  }
  // Validate first; refuse to construct a filter with degenerate rules so the
  // host's plugin construction errors out loud at startup.
  rules.forEach((rule, index) => {
    const hasRoute =
      rule.routePattern !== undefined && rule.routePattern.length > 0;
    const hasRuleIds = rule.ruleIds !== undefined && rule.ruleIds.length > 0;
    if (!hasRoute && !hasRuleIds) {
      throw new Error(
        `seogeoPlugin: suppressions[${index}] is empty — set routePattern, ruleIds, or both`,
      );
    }
  });
  const compiled = rules.map((rule) => ({
    matchRoute: rule.routePattern === undefined
      ? matchAlways
      : globToMatcher(rule.routePattern),
    ruleIds:
      rule.ruleIds === undefined || rule.ruleIds.length === 0
        ? null
        : new Set(rule.ruleIds),
  }));
  return {
    apply(route, findings) {
      return findings.filter((finding) => !compiled.some((rule) => {
        if (!rule.matchRoute(route)) return false;
        if (rule.ruleIds !== null && !rule.ruleIds.has(finding.rule_id)) {
          return false;
        }
        return true;
      }));
    },
  };
}

type RouteMatcher = (route: string) => boolean;

const matchAlways: RouteMatcher = () => true;

/// Convert a glob to a route matcher. Implementation note: we go via regex
/// because the alternative (hand-rolled state machine) is error-prone for
/// `**` interaction with `/`. The regex is anchored on both sides.
///
/// Order matters in the substitution: `**` MUST be replaced before `*` or
/// the single-`*` rule will eat both stars.
function globToMatcher(pattern: string): RouteMatcher {
  // Preserve the leading slash convention: a pattern like "/about" should
  // match exactly the route "/about", and the regex must be anchored.
  const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&");
  // Three-pass substitution for the glob metacharacters:
  //   1. ** → placeholder (so we can distinguish from *)
  //   2. *  → [^/]*
  //   3. placeholder → .*
  // The `?` is straightforward and goes last.
  const DOUBLE_STAR_TOKEN = ""; // any char never appearing in a route
  let body = escaped
    .replace(/\*\*/g, DOUBLE_STAR_TOKEN)
    .replace(/\*/g, "[^/]*")
    .replace(new RegExp(DOUBLE_STAR_TOKEN, "g"), ".*")
    .replace(/\?/g, "[^/]");
  const re = new RegExp(`^${body}$`);
  return (route) => re.test(route);
}
