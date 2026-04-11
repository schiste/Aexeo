use super::ConfigFieldDoc;

pub fn config_field_docs() -> &'static [ConfigFieldDoc] {
    &[
        ConfigFieldDoc {
            key: "site_url",
            default_value: "null",
            description: "Canonical base URL used for generation, canonical autofix, and robots output.",
        },
        ConfigFieldDoc {
            key: "source_dir",
            default_value: "`.`",
            description: "Optional build-output directory beneath the repository root.",
        },
        ConfigFieldDoc {
            key: "profile",
            default_value: "`generic`",
            description: "Built-in policy profile. `chau7` enables stricter feature-page expectations.",
        },
        ConfigFieldDoc {
            key: "adapter",
            default_value: "`auto`",
            description: "Adapter selection mode. `auto` chooses the highest-priority matching adapter.",
        },
        ConfigFieldDoc {
            key: "plugins",
            default_value: "(none)",
            description: "Modules that extend rules or adapters.",
        },
        ConfigFieldDoc {
            key: "canonical_style",
            default_value: "`extensionless`",
            description: "Preferred internal canonical style. `extensionless` expects clean routes.",
        },
        ConfigFieldDoc {
            key: "extends",
            default_value: "(none)",
            description: "Optional parent config file or files to merge before the current config.",
        },
        ConfigFieldDoc {
            key: "audit_log_limit",
            default_value: "`5`",
            description: "Number of audit artifacts retained per command, including history pruning behavior.",
        },
        ConfigFieldDoc {
            key: "browser_engine",
            default_value: "`auto`",
            description: "Default crawl engine. `auto` currently resolves to the native HTTP crawler; Playwright remains an explicit opt-in compatibility mode.",
        },
        ConfigFieldDoc {
            key: "browser_wait_until",
            default_value: "`networkidle`",
            description: "Playwright navigation wait strategy for browser-backed crawl mode.",
        },
        ConfigFieldDoc {
            key: "baseline_file",
            default_value: "`.seogeo-baseline.json`",
            description: "Default audit baseline path used for verification and diff workflows.",
        },
        ConfigFieldDoc {
            key: "max_workers",
            default_value: "`4`",
            description: "Worker count used for parallel file parsing and selected analysis tasks.",
        },
        ConfigFieldDoc {
            key: "enable_cache",
            default_value: "`true`",
            description: "Whether persistent parse and crawl caches may be used.",
        },
        ConfigFieldDoc {
            key: "cache_dir",
            default_value: "`.seogeo-cache`",
            description: "Directory for persistent seogeo caches.",
        },
        ConfigFieldDoc {
            key: "cache_ttl_seconds",
            default_value: "`3600`",
            description: "Maximum age for reusable crawl cache entries.",
        },
        ConfigFieldDoc {
            key: "crawl_headers",
            default_value: "(none)",
            description: "Extra HTTP headers applied to runtime crawl requests.",
        },
        ConfigFieldDoc {
            key: "crawl_cookies",
            default_value: "(none)",
            description: "Cookies injected into browser/runtime crawl sessions.",
        },
        ConfigFieldDoc {
            key: "crawl_basic_auth",
            default_value: "(none)",
            description: "Basic auth credentials for runtime crawl sessions.",
        },
        ConfigFieldDoc {
            key: "crawl_capture_trace",
            default_value: "`false`",
            description: "Whether browser crawl should capture a Playwright trace artifact.",
        },
        ConfigFieldDoc {
            key: "crawl_seeds",
            default_value: "(none)",
            description: "Additional route seeds for runtime crawl, such as `/pricing` or `/docs/getting-started`.",
        },
        ConfigFieldDoc {
            key: "crawl_include_patterns",
            default_value: "(none)",
            description: "Optional route substrings used to constrain runtime crawl scope.",
        },
        ConfigFieldDoc {
            key: "crawl_exclude_patterns",
            default_value: "(none)",
            description: "Route substrings excluded from runtime crawl.",
        },
        ConfigFieldDoc {
            key: "crawl_use_sitemap",
            default_value: "`true`",
            description: "Whether runtime crawl should seed additional routes from sitemap.xml when available.",
        },
        ConfigFieldDoc {
            key: "crawl_capture_screenshot",
            default_value: "`false`",
            description: "Whether browser crawl should save page screenshots.",
        },
        ConfigFieldDoc {
            key: "crawl_capture_console",
            default_value: "`false`",
            description: "Whether browser crawl should persist console output.",
        },
        ConfigFieldDoc {
            key: "crawl_capture_network",
            default_value: "`false`",
            description: "Whether browser crawl should persist network request logs.",
        },
        ConfigFieldDoc {
            key: "crawl_artifact_dir",
            default_value: "`.seogeo-reports/crawl-artifacts`",
            description: "Directory used for crawl artifacts such as traces, screenshots, console logs, and network logs.",
        },
        ConfigFieldDoc {
            key: "ignore_rules",
            default_value: "(none)",
            description: "Rule IDs to suppress after evaluation.",
        },
        ConfigFieldDoc {
            key: "ignore_paths",
            default_value: "(none)",
            description: "Glob-like path patterns to suppress after evaluation.",
        },
        ConfigFieldDoc {
            key: "severity_overrides",
            default_value: "(none)",
            description: "Per-rule severity overrides applied after rules run.",
        },
        ConfigFieldDoc {
            key: "suppressions",
            default_value: "(none)",
            description: "Explicit reviewable suppressions with rule, path pattern, reason, and optional expiry.",
        },
        ConfigFieldDoc {
            key: "orphan_exclude",
            default_value: "`404.html`",
            description: "Routes or filenames excluded from orphan detection.",
        },
        ConfigFieldDoc {
            key: "repeatable_data_ui",
            default_value: "`[\"card\", \"item\", \"entry\", ...]`",
            description: "data-ui labels treated as intentionally repeatable list-item markers rather than page-unique identifiers.",
        },
        ConfigFieldDoc {
            key: "utility_route_patterns",
            default_value: "`[\"search\", \"admin\", \"preview\", \"internal\"]`",
            description: "Route substrings used to classify utility pages for robots-policy nuance.",
        },
        ConfigFieldDoc {
            key: "route_policy_overrides",
            default_value: "(none)",
            description: "Per-route robots policy overrides such as allowing canonical+noindex or nofollow.",
        },
        ConfigFieldDoc {
            key: "min_inbound_links",
            default_value: "`1`",
            description: "Minimum inbound internal links before LNK004 triggers.",
        },
        ConfigFieldDoc {
            key: "link_suggestion_count",
            default_value: "`3`",
            description: "Number of candidate internal-link suggestions to produce per weakly linked page.",
        },
        ConfigFieldDoc {
            key: "enable_link_autofix",
            default_value: "`false`",
            description: "Whether safe fix mode may insert a generated related-links section into pages.",
        },
        ConfigFieldDoc {
            key: "related_links_heading",
            default_value: "`Related pages`",
            description: "Heading text used when link autofix inserts a related-links section.",
        },
        ConfigFieldDoc {
            key: "min_page_size",
            default_value: "`500`",
            description: "Minimum visible text length before CNT001 triggers.",
        },
        ConfigFieldDoc {
            key: "required_feature_markers",
            default_value: "`Related features`",
            description: "Literal strings required on feature-like pages before CNT002 triggers.",
        },
        ConfigFieldDoc {
            key: "min_block_text_length",
            default_value: "`120`",
            description: "Minimum visible text length for a semantic content block before GEO chunk-thinness triggers.",
        },
        ConfigFieldDoc {
            key: "min_answer_blocks",
            default_value: "`2`",
            description: "Minimum number of answer-oriented blocks before GEO answerability triggers.",
        },
        ConfigFieldDoc {
            key: "require_fact_consistency",
            default_value: "`true`",
            description: "Whether title/H1/OpenGraph/schema facts should align on a page.",
        },
        ConfigFieldDoc {
            key: "required_schema_types",
            default_value: "(none)",
            description: "JSON-LD @type values expected on route pages.",
        },
        ConfigFieldDoc {
            key: "required_schema_families",
            default_value: "(none)",
            description: "Schema families that should be validated when present or required by policy.",
        },
        ConfigFieldDoc {
            key: "require_breadcrumb_schema",
            default_value: "`false`",
            description: "Whether nested pages must emit BreadcrumbList schema.",
        },
        ConfigFieldDoc {
            key: "require_schema_title_alignment",
            default_value: "`true`",
            description: "Whether JSON-LD name/headline values must align with visible title or H1.",
        },
        ConfigFieldDoc {
            key: "require_html_lang",
            default_value: "`true`",
            description: "Whether indexable pages must declare a root html lang attribute.",
        },
        ConfigFieldDoc {
            key: "require_hreflang_self",
            default_value: "`false`",
            description: "Whether pages using hreflang alternates must include a self-referencing hreflang.",
        },
        ConfigFieldDoc {
            key: "require_meta_robots_consistency",
            default_value: "`true`",
            description: "Whether noindex pages in sitemap should be reported as inconsistent.",
        },
        ConfigFieldDoc {
            key: "require_open_graph",
            default_value: "`true`",
            description: "Whether og:title, og:description, and og:type are required.",
        },
        ConfigFieldDoc {
            key: "require_twitter_card",
            default_value: "`true`",
            description: "Whether twitter:card is required.",
        },
        ConfigFieldDoc {
            key: "default_twitter_card",
            default_value: "`summary`",
            description: "Fallback twitter:card value used by safe HTML autofix.",
        },
        ConfigFieldDoc {
            key: "require_social_images",
            default_value: "`false`",
            description: "Whether shared pages must provide og:image and optionally twitter:image.",
        },
        ConfigFieldDoc {
            key: "require_twitter_image",
            default_value: "`false`",
            description: "Whether twitter:image is required in addition to og:image.",
        },
        ConfigFieldDoc {
            key: "require_robots_sitemap",
            default_value: "`true`",
            description: "Whether robots.txt must declare the sitemap URL.",
        },
        ConfigFieldDoc {
            key: "weak_anchor_text",
            default_value: "`click here`, `here`, `learn more`, `more`, `read more`",
            description: "Anchor phrases treated as weak for internal-link quality checks.",
        },
        ConfigFieldDoc {
            key: "plugin_settings",
            default_value: "(none)",
            description: "Plugin-specific configuration grouped by plugin namespace.",
        },
        ConfigFieldDoc {
            key: "typecheck_command",
            default_value: "`cargo check`",
            description: "Command used for static type checking in internal quality workflows.",
        },
        ConfigFieldDoc {
            key: "coverage_threshold",
            default_value: "`85`",
            description: "Minimum expected test coverage percentage for internal quality workflows.",
        },
        ConfigFieldDoc {
            key: "complexity_threshold",
            default_value: "`12`",
            description: "Maximum allowed AST branch complexity score per public function.",
        },
        ConfigFieldDoc {
            key: "performance_budget_file",
            default_value: "`performance-budget.json`",
            description: "Path to a JSON file describing runtime performance budgets.",
        },
    ]
}
