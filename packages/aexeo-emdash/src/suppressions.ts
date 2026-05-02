// Editor-workflow suppressions for plugin findings.
//
// Suppressions are a host policy, not an engine concern. The CLI deliberately
// stays strict — its findings reflect the canonical audit. The plugin's
// suppressions live one level up: they let editors silence specific routes,
// rules, collections, or document statuses that are intentional violations
// the host has decided not to fix (legal pages with non-standard SEO;
// localized routes where a rule doesn't apply; draft documents whose
// findings shouldn't surface yet).
//
// Filtering happens BEFORE findings are persisted to KV. Suppressed findings
// never reach the dashboard, the /findings page, or the per-document panel.
// This is intentional — if a suppression were applied at render time only,
// editors would see noise during refresh and have to ignore it manually.

import type { Finding } from "./types.js";

/// Single suppression rule. At least one selector must be set; `{}` is
/// rejected at config-load time as a likely editor mistake.
///
/// Matching is **AND across selectors, OR across rules**: a finding is
/// suppressed when at least one rule matches it; a rule matches when every
/// selector it sets matches the finding's context. Absent selectors are
/// treated as "match anything."
///
/// Selectors:
///   - `routePattern` — glob over the document's route. `*` matches non-`/`
///     chars; `**` matches across `/`; `?` matches one non-`/` char.
///     Patterns are anchored. `/about` matches only `/about`, not
///     `/about/team` — use `/about/**` for a subtree.
///   - `ruleIds` — finding's `rule_id` must be in the set.
///   - `collections` — document's emdash collection must be in the set.
///     Only meaningful for per-document findings; sitewide findings have
///     no single collection and are never matched by this selector.
///   - `statuses` — document's emdash status (e.g. `"draft"`,
///     `"published"`) must be in the set. Same caveat: sitewide findings
///     don't carry a status.
export interface Suppression {
  routePattern?: string;
  ruleIds?: readonly string[];
  collections?: readonly string[];
  statuses?: readonly string[];
}

/// Per-document context the filter consumes. The plugin assembles this from
/// the document's stored metadata before calling `apply`. Sitewide findings
/// pass `route: "*"` with no collection/status — selectors that require those
/// fields automatically don't match, which is the right behavior (sitewide
/// findings are by definition cross-document).
export interface SuppressionContext {
  route: string;
  collection?: string;
  status?: string;
}

/// Pre-compiled filter. `apply` returns the input findings minus any matching
/// the configured suppressions. Empty input or empty rules → identity.
export interface SuppressionFilter {
  apply(
    context: SuppressionContext,
    findings: readonly Finding[],
  ): Finding[];
}

const NOOP_FILTER: SuppressionFilter = {
  apply: (_context, findings) => [...findings],
};

/// Validates and compiles suppressions to a fast filter. Throws when a rule
/// has no selectors set — that's a kill-switch for all findings everywhere
/// and almost certainly an editor mistake.
export function compileSuppressions(
  rules: readonly Suppression[] | undefined,
): SuppressionFilter {
  if (rules === undefined || rules.length === 0) {
    return NOOP_FILTER;
  }
  // Validate first so a malformed rule fails the host's plugin construction
  // loudly rather than silently dropping every finding at runtime.
  rules.forEach((rule, index) => {
    const hasRoute =
      rule.routePattern !== undefined && rule.routePattern.length > 0;
    const hasRuleIds = rule.ruleIds !== undefined && rule.ruleIds.length > 0;
    const hasCollections =
      rule.collections !== undefined && rule.collections.length > 0;
    const hasStatuses =
      rule.statuses !== undefined && rule.statuses.length > 0;
    if (!hasRoute && !hasRuleIds && !hasCollections && !hasStatuses) {
      throw new Error(
        `aexeoPlugin: suppressions[${index}] is empty — set at least one of routePattern, ruleIds, collections, statuses`,
      );
    }
  });
  const compiled = rules.map((rule) => ({
    matchRoute:
      rule.routePattern === undefined
        ? matchAlways
        : globToMatcher(rule.routePattern),
    ruleIds:
      rule.ruleIds === undefined || rule.ruleIds.length === 0
        ? null
        : new Set(rule.ruleIds),
    collections:
      rule.collections === undefined || rule.collections.length === 0
        ? null
        : new Set(rule.collections),
    statuses:
      rule.statuses === undefined || rule.statuses.length === 0
        ? null
        : new Set(rule.statuses),
  }));
  return {
    apply(context, findings) {
      return findings.filter(
        (finding) =>
          !compiled.some((rule) => {
            if (!rule.matchRoute(context.route)) return false;
            if (rule.ruleIds !== null && !rule.ruleIds.has(finding.rule_id)) {
              return false;
            }
            // Collection/status selectors only match when the context
            // carries the corresponding field. Sitewide findings (route
            // "*") have neither, so a rule that uses these selectors
            // will not match them — which is the right behavior, since
            // sitewide findings are inherently cross-document.
            if (rule.collections !== null) {
              if (
                context.collection === undefined ||
                !rule.collections.has(context.collection)
              ) {
                return false;
              }
            }
            if (rule.statuses !== null) {
              if (
                context.status === undefined ||
                !rule.statuses.has(context.status)
              ) {
                return false;
              }
            }
            return true;
          }),
      );
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
  const body = escaped
    .replace(/\*\*/g, DOUBLE_STAR_TOKEN)
    .replace(/\*/g, "[^/]*")
    .replace(new RegExp(DOUBLE_STAR_TOKEN, "g"), ".*")
    .replace(/\?/g, "[^/]");
  const re = new RegExp(`^${body}$`);
  return (route) => re.test(route);
}
