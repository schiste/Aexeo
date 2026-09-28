use aexeo_contracts::{ConfidenceLevel, Layer, RuleClass, RuleLayers, RuleMetadata};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDescriptor {
    pub rule_id: &'static str,
    pub summary: &'static str,
    /// Whether the finding describes the whole site rather than one page.
    ///
    /// Scope used to live in a second prefix table in `reporting.rs`, which
    /// predated the `AGT`, `SRF`, `EDT`, and `A11Y` families and silently
    /// reported their sitewide findings as page-scoped. Recording it here
    /// keeps the registry the single source of truth.
    pub sitewide: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleGroupDefinition {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub rules: &'static [RuleDescriptor],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub priority: i32,
}

impl RuleDescriptor {
    pub fn metadata(&self) -> RuleMetadata {
        rule_metadata_for_id(self.rule_id)
    }

    pub fn layers(&self) -> RuleLayers {
        rule_layers_for_id(self.rule_id)
    }
}

/// Metadata for an explicitly known rule-ID prefix, or `None`.
///
/// Same shape and rationale as [`explicit_layers_for_prefix`]: the returned
/// option distinguishes "this family is registered" from "this fell through
/// to the default", which a bare value cannot.
fn explicit_metadata_for_prefix(prefix: &str) -> Option<RuleMetadata> {
    let value = match prefix {
        "SEO" => RuleMetadata {
            class: RuleClass::Hard,
            confidence: ConfidenceLevel::High,
        },
        "LNK" => RuleMetadata {
            class: RuleClass::Hard,
            confidence: ConfidenceLevel::High,
        },
        "MAP" => RuleMetadata {
            class: RuleClass::Hard,
            confidence: ConfidenceLevel::High,
        },
        "ROB" => RuleMetadata {
            class: RuleClass::Policy,
            confidence: ConfidenceLevel::High,
        },
        "SOC" => RuleMetadata {
            class: RuleClass::Hard,
            confidence: ConfidenceLevel::High,
        },
        "SCH" => RuleMetadata {
            class: RuleClass::Policy,
            confidence: ConfidenceLevel::Medium,
        },
        "LLM" => RuleMetadata {
            class: RuleClass::Policy,
            confidence: ConfidenceLevel::Medium,
        },
        "SRF" => RuleMetadata {
            class: RuleClass::Policy,
            confidence: ConfidenceLevel::High,
        },
        "CNT" => RuleMetadata {
            class: RuleClass::Policy,
            confidence: ConfidenceLevel::Medium,
        },
        "EDT" => RuleMetadata {
            class: RuleClass::Policy,
            confidence: ConfidenceLevel::Medium,
        },
        "GEO" => RuleMetadata {
            class: RuleClass::Heuristic,
            confidence: ConfidenceLevel::Medium,
        },
        "CRW" => RuleMetadata {
            class: RuleClass::Heuristic,
            confidence: ConfidenceLevel::Low,
        },
        "DEP" => RuleMetadata {
            class: RuleClass::Policy,
            confidence: ConfidenceLevel::High,
        },
        "QLT" => RuleMetadata {
            class: RuleClass::Policy,
            confidence: ConfidenceLevel::Medium,
        },
        // A11Y: static accessibility rules. Most low-hanging static
        // checks (missing alt, empty link, duplicate id) are
        // deterministic HTML-spec facts — Hard / High by default.
        // Heuristic rules (heading-jump, alt-equals-filename) can
        // be downgraded per-rule.
        "A11Y" => RuleMetadata {
            class: RuleClass::Hard,
            confidence: ConfidenceLevel::High,
        },
        // AGT: agent-discovery checks (api-catalog, mcp server-card,
        // future agent-card / agent-skills). Policy class because
        // they advocate for nascent specs (RFC 9727, SEP-1649) that
        // sites can defensibly skip — but Medium confidence, not
        // High, because spec adoption is uneven and a missing file
        // doesn't always mean a misconfiguration.
        "AGT" => RuleMetadata {
            class: RuleClass::Policy,
            confidence: ConfidenceLevel::Medium,
        },
        _ => return None,
    };
    Some(value)
}

/// Metadata by rule-ID prefix, with a documented default for unrecognised
/// prefixes. Unknown families are treated as medium-confidence heuristics,
/// which is the conservative classification: it keeps an unregistered rule
/// visible without claiming it is a hard spec violation.
pub fn metadata_for_prefix(prefix: &str) -> RuleMetadata {
    explicit_metadata_for_prefix(prefix).unwrap_or(RuleMetadata {
        class: RuleClass::Heuristic,
        confidence: ConfidenceLevel::Medium,
    })
}

/// Whether `prefix` has an explicit entry in the metadata table.
pub fn is_known_metadata_prefix(prefix: &str) -> bool {
    explicit_metadata_for_prefix(prefix).is_some()
}

/// Strip the trailing numeric suffix from a rule id to recover the
/// family prefix. Rule ids follow the shape `<PREFIX><NNN>` where the
/// prefix is letters-and-digits but never *ends* in a digit
/// (e.g. `SEO001`, `FACTS003`, `A11Y001`). The earlier
/// `take_while(uppercase)` form broke on alphanumeric prefixes like
/// `A11Y` (it would yield `"A"` and silently fall to the default
/// citability layer, losing the accessibility mapping).
pub fn rule_prefix(rule_id: &str) -> &str {
    rule_id.trim_end_matches(|c: char| c.is_ascii_digit())
}

pub fn rule_metadata_for_id(rule_id: &str) -> RuleMetadata {
    let prefix = rule_prefix(rule_id);
    let mut metadata = metadata_for_prefix(prefix);
    if rule_id.starts_with("GEO0") {
        metadata.confidence = ConfidenceLevel::Low;
    }
    if matches!(
        rule_id,
        "GEO001" | "GEO002" | "GEO003" | "GEO004" | "GEO005" | "GEO006"
    ) {
        metadata.confidence = ConfidenceLevel::High;
    }
    metadata
}

/// Layer assignment for an explicitly known rule-ID prefix.
///
/// Returns `None` for an unrecognised prefix so the caller decides what to
/// do. That distinction matters: several real prefixes map to
/// `Layer::Citability` legitimately, so a returned value cannot tell you
/// whether a family was registered or simply fell through to the default.
/// `A11Y`, `SRF`, `AGT`, and `EDT` all once fell through, and nothing
/// noticed until reports started labelling those findings "Other".
fn explicit_layers_for_prefix(prefix: &str) -> Option<RuleLayers> {
    let value = match prefix {
        // SEO: title / description / meta. Most directly help the
        // generator decide what's worth citing once retrieved.
        // Secondary retrievability because search engines also use these.
        "SEO" => RuleLayers::with_secondaries(Layer::Citability, vec![Layer::Retrievability]),
        // LNK: link integrity. Broken or missing links break crawl /
        // graph traversal — purely retrievability.
        "LNK" => RuleLayers::primary_only(Layer::Retrievability),
        // MAP: sitemap. Pure retrievability.
        "MAP" => RuleLayers::primary_only(Layer::Retrievability),
        // ROB: robots.txt and meta-robots. Pure retrievability.
        "ROB" => RuleLayers::primary_only(Layer::Retrievability),
        // SOC: Open Graph / Twitter. Snippet content shown in shared
        // contexts. Citability primary (it's how previews compose);
        // retrievability secondary (some engines weigh OG).
        "SOC" => RuleLayers::with_secondaries(Layer::Citability, vec![Layer::Retrievability]),
        // SCH: schema.org / JSON-LD. Citability primary (machine-legible
        // structure for citation); absorbability secondary (entities
        // and relations the generator can lift).
        "SCH" => RuleLayers::with_secondaries(Layer::Citability, vec![Layer::Absorbability]),
        // LLM: llms.txt and friends. Absorbability primary (cite-ready
        // content for LLMs); retrievability secondary (a discovered
        // surface).
        "LLM" => RuleLayers::with_secondaries(Layer::Absorbability, vec![Layer::Retrievability]),
        // SRF: surface graph (machine-readable surfaces, manifests,
        // discovery). Retrievability primary; absorbability secondary
        // because mirrors and llms feed both.
        "SRF" => RuleLayers::with_secondaries(Layer::Retrievability, vec![Layer::Absorbability]),
        // CNT: content rules. Citability primary (whether the content
        // is structured well enough to cite); absorbability secondary.
        "CNT" => RuleLayers::with_secondaries(Layer::Citability, vec![Layer::Absorbability]),
        // EDT: explicit editorial briefs and their rendered answer, question,
        // and evidence markers. These are citation-shape policy checks.
        "EDT" => RuleLayers::with_secondaries(Layer::Citability, vec![Layer::Absorbability]),
        // GEO: structural and content patterns specifically for
        // generative engines. Citability primary; absorbability
        // secondary.
        "GEO" => RuleLayers::with_secondaries(Layer::Citability, vec![Layer::Absorbability]),
        // CRW: crawl-state diagnostics. Pure retrievability.
        "CRW" => RuleLayers::primary_only(Layer::Retrievability),
        // DEP: deprecated / migration warnings. Citability default
        // (most are about page metadata).
        "DEP" => RuleLayers::primary_only(Layer::Citability),
        // QLT: quality / repo-config. Citability default — these are
        // typically about content quality.
        "QLT" => RuleLayers::primary_only(Layer::Citability),
        // FACTS: truth manifest rules. Pure entity legitimacy.
        "FACTS" => RuleLayers::primary_only(Layer::EntityLegitimacy),
        // A11Y: static accessibility. The fifth axis is its own
        // primary by default — humans-using-the-page is the goal.
        // Per-rule overrides add GEO secondaries where the A11Y
        // signal genuinely feeds retrievability or citability
        // (alt text → image search, landmarks → snippet shape).
        "A11Y" => RuleLayers::primary_only(Layer::Accessibility),
        // AGT: agent-discovery files (api-catalog, mcp server-card).
        // Retrievability primary — these files exist so agents can
        // *find* the site's machine-readable surfaces. Absorbability
        // secondary because the surfaces they point at (mcp server,
        // api catalog) feed agent absorption of the site's content
        // and capabilities.
        "AGT" => RuleLayers::with_secondaries(Layer::Retrievability, vec![Layer::Absorbability]),
        _ => return None,
    };
    Some(value)
}

/// Layer assignment by rule-ID prefix, with a documented default for
/// unrecognised prefixes.
///
/// Unknown prefixes default to citability (most rules are about making the
/// page worth citing) rather than panicking. Use
/// [`explicit_layers_for_prefix`] when you need to know whether a family
/// was actually registered.
pub fn layers_for_prefix(prefix: &str) -> RuleLayers {
    explicit_layers_for_prefix(prefix)
        .unwrap_or_else(|| RuleLayers::primary_only(Layer::Citability))
}

/// Whether `prefix` has an explicit entry in the layer table.
pub fn is_known_layer_prefix(prefix: &str) -> bool {
    explicit_layers_for_prefix(prefix).is_some()
}

/// Compile-time exhaustiveness guard for [`Layer`].
///
/// Exists so a new layer variant cannot be added without every exhaustive
/// `match` over layers in the crate being revisited. Deliberately has no
/// runtime effect.
#[cfg(test)]
fn assert_exhaustive_layer(layer: Layer) {
    match layer {
        Layer::Retrievability
        | Layer::Citability
        | Layer::Absorbability
        | Layer::EntityLegitimacy
        | Layer::Accessibility => {}
    }
}

/// Per-rule layer overrides. Most rules pick up the prefix default.
/// The overrides here are for rules whose individual purpose differs
/// from their family's typical layer.
pub fn rule_layers_for_id(rule_id: &str) -> RuleLayers {
    let mut layers = layers_for_prefix(rule_prefix(rule_id));

    // Schema rules whose primary effect is retrievability / discovery
    // rather than citation-shape:
    //   SCH011 — home page sitewide context (retrievability primary)
    //   SCH015 — search page SearchAction (retrievability primary)
    if matches!(rule_id, "SCH011" | "SCH015") {
        layers = RuleLayers::with_secondaries(Layer::Retrievability, vec![Layer::Citability]);
    }
    // GEO009 (fact alignment across title/H1/OG/JSON-LD) is pure
    // citability — it doesn't help absorbability.
    if rule_id == "GEO009" {
        layers = RuleLayers::primary_only(Layer::Citability);
    }
    // SRF005 / SRF006 (mirror discoverability) are absorbability primary
    // because the mirror is the content the generator absorbs.
    if matches!(rule_id, "SRF005" | "SRF006") {
        layers = RuleLayers::with_secondaries(Layer::Absorbability, vec![Layer::Retrievability]);
    }

    // A11Y per-rule cross-tag secondaries. Most A11Y rules are
    // primary-only on Accessibility (the prefix default) — these
    // are the ones whose signal genuinely feeds a GEO axis too.
    match rule_id {
        // Alt text: image search and crawlers use it. Decorative
        // images don't fire here, so when this rule fires the
        // missing alt is also a retrievability gap.
        "A11Y001" | "A11Y006" => {
            layers =
                RuleLayers::with_secondaries(Layer::Accessibility, vec![Layer::Retrievability]);
        }
        // Empty links break the link graph that crawlers and AI
        // engines walk; landmarks shape the snippets engines
        // extract. Both feed citability.
        "A11Y002" | "A11Y005" => {
            layers = RuleLayers::with_secondaries(Layer::Accessibility, vec![Layer::Citability]);
        }
        // Heading hierarchy is heavily used by snippet selection.
        "A11Y004" => {
            layers = RuleLayers::with_secondaries(Layer::Accessibility, vec![Layer::Citability]);
        }
        _ => {}
    }

    layers
}

pub fn rule_descriptor_for_id(rule_id: &str) -> Option<&'static RuleDescriptor> {
    builtin_rule_groups()
        .iter()
        .flat_map(|group| group.rules.iter())
        .find(|descriptor| descriptor.rule_id == rule_id)
}

pub fn builtin_rule_groups() -> &'static [RuleGroupDefinition] {
    &[
        RuleGroupDefinition {
            name: "html",
            title: "HTML Metadata",
            description: "",
            rules: &[
                RuleDescriptor {
                    rule_id: "SEO001",
                    sitewide: false,
                    summary: "missing <title>",
                },
                RuleDescriptor {
                    rule_id: "SEO002",
                    sitewide: false,
                    summary: "missing meta description",
                },
                RuleDescriptor {
                    rule_id: "SEO004",
                    sitewide: false,
                    summary: "missing canonical",
                },
                RuleDescriptor {
                    rule_id: "SEO005",
                    sitewide: false,
                    summary: "missing <h1>",
                },
                RuleDescriptor {
                    rule_id: "SEO006",
                    sitewide: false,
                    summary: "multiple <h1> tags",
                },
                RuleDescriptor {
                    rule_id: "SEO007",
                    sitewide: false,
                    summary: "missing root html lang attribute",
                },
                RuleDescriptor {
                    rule_id: "SEO008",
                    sitewide: false,
                    summary: "page has hreflang alternates but no self-referencing hreflang",
                },
                RuleDescriptor {
                    rule_id: "SEO009",
                    sitewide: false,
                    summary: "hreflang alternate points to a missing internal path",
                },
                RuleDescriptor {
                    rule_id: "SEO010",
                    sitewide: false,
                    summary: "invalid hreflang locale code",
                },
                RuleDescriptor {
                    rule_id: "SEO011",
                    sitewide: true,
                    summary: "hreflang cluster is missing x-default",
                },
                RuleDescriptor {
                    rule_id: "SEO012",
                    sitewide: false,
                    summary: "hreflang alternate is not reciprocally linked",
                },
                RuleDescriptor {
                    rule_id: "SEO013",
                    sitewide: false,
                    summary: "page suppresses snippets via nosnippet",
                },
                RuleDescriptor {
                    rule_id: "SEO014",
                    sitewide: false,
                    summary: "page restricts snippets via max-snippet",
                },
                RuleDescriptor {
                    rule_id: "SEO015",
                    sitewide: false,
                    summary: "page uses data-nosnippet blocks",
                },
                RuleDescriptor {
                    rule_id: "SEO016",
                    sitewide: false,
                    summary: "page canonicals to another crawlable route",
                },
                RuleDescriptor {
                    rule_id: "SEO017",
                    sitewide: true,
                    summary: "duplicate title and meta description cluster",
                },
            ],
        },
        RuleGroupDefinition {
            name: "links",
            title: "Internal Links",
            description: "",
            rules: &[
                RuleDescriptor {
                    rule_id: "LNK001",
                    sitewide: false,
                    summary: "broken internal link",
                },
                RuleDescriptor {
                    rule_id: "LNK002",
                    sitewide: false,
                    summary: "orphan page",
                },
                RuleDescriptor {
                    rule_id: "LNK003",
                    sitewide: false,
                    summary: "weak internal anchor text",
                },
                RuleDescriptor {
                    rule_id: "LNK004",
                    sitewide: false,
                    summary: "insufficient inbound internal links",
                },
            ],
        },
        RuleGroupDefinition {
            name: "sitemap",
            title: "Sitemaps",
            description: "",
            rules: &[
                RuleDescriptor {
                    rule_id: "MAP001",
                    sitewide: true,
                    summary: "missing sitemap.xml",
                },
                RuleDescriptor {
                    rule_id: "MAP002",
                    sitewide: true,
                    summary: "invalid sitemap XML",
                },
                RuleDescriptor {
                    rule_id: "MAP003",
                    sitewide: true,
                    summary: "empty sitemap set",
                },
                RuleDescriptor {
                    rule_id: "MAP004",
                    sitewide: true,
                    summary: "canonical missing from sitemap coverage",
                },
                RuleDescriptor {
                    rule_id: "MAP005",
                    sitewide: true,
                    summary: "sitemap.xml exists but is empty",
                },
                RuleDescriptor {
                    rule_id: "MAP006",
                    sitewide: true,
                    summary: "sitemap endpoint returned HTML instead of sitemap XML",
                },
                RuleDescriptor {
                    rule_id: "MAP007",
                    sitewide: true,
                    summary: "sitemap endpoint exists but is not recognizable sitemap XML",
                },
                RuleDescriptor {
                    rule_id: "MAP008",
                    sitewide: true,
                    summary: "sitemap.xml is missing lastmod values",
                },
                RuleDescriptor {
                    rule_id: "MAP009",
                    sitewide: true,
                    summary: "sitemap.xml has invalid lastmod values",
                },
            ],
        },
        RuleGroupDefinition {
            name: "robots",
            title: "Robots",
            description: "",
            rules: &[
                RuleDescriptor {
                    rule_id: "ROB001",
                    sitewide: true,
                    summary: "missing robots.txt",
                },
                RuleDescriptor {
                    rule_id: "ROB002",
                    sitewide: true,
                    summary: "missing Sitemap: declaration in robots.txt",
                },
                RuleDescriptor {
                    rule_id: "ROB003",
                    sitewide: true,
                    summary: "robots.txt blocks the whole site for User-agent: *",
                },
                RuleDescriptor {
                    rule_id: "ROB004",
                    sitewide: true,
                    summary: "page is in sitemap but declares noindex in meta robots",
                },
                RuleDescriptor {
                    rule_id: "ROB005",
                    sitewide: true,
                    summary: "page declares both canonical and noindex",
                },
                RuleDescriptor {
                    rule_id: "ROB006",
                    sitewide: true,
                    summary: "page declares nofollow",
                },
                RuleDescriptor {
                    rule_id: "ROB007",
                    sitewide: true,
                    summary: "robots.txt may overblock crawl budget",
                },
                RuleDescriptor {
                    rule_id: "ROB008",
                    sitewide: true,
                    summary: "page is in sitemap but declares noindex in X-Robots-Tag",
                },
                RuleDescriptor {
                    rule_id: "ROB010",
                    sitewide: true,
                    summary: "robots.txt has no AI-bot User-agent directives",
                },
                RuleDescriptor {
                    rule_id: "ROB011",
                    sitewide: true,
                    summary: "robots.txt has no Content-Signal directives",
                },
            ],
        },
        RuleGroupDefinition {
            name: "social",
            title: "Social Metadata",
            description: "",
            rules: &[
                RuleDescriptor {
                    rule_id: "SOC001",
                    sitewide: false,
                    summary: "missing og:title",
                },
                RuleDescriptor {
                    rule_id: "SOC002",
                    sitewide: false,
                    summary: "missing og:description",
                },
                RuleDescriptor {
                    rule_id: "SOC003",
                    sitewide: false,
                    summary: "missing og:type",
                },
                RuleDescriptor {
                    rule_id: "SOC004",
                    sitewide: false,
                    summary: "missing twitter:card",
                },
                RuleDescriptor {
                    rule_id: "SOC005",
                    sitewide: false,
                    summary: "og:url does not match canonical",
                },
                RuleDescriptor {
                    rule_id: "SOC006",
                    sitewide: false,
                    summary: "missing og:image",
                },
                RuleDescriptor {
                    rule_id: "SOC007",
                    sitewide: false,
                    summary: "missing twitter:image",
                },
                RuleDescriptor {
                    rule_id: "SOC008",
                    sitewide: false,
                    summary: "social image points to a missing internal asset",
                },
                RuleDescriptor {
                    rule_id: "SOC009",
                    sitewide: false,
                    summary: "twitter:card is `summary`; consider `summary_large_image`",
                },
                RuleDescriptor {
                    rule_id: "SOC010",
                    sitewide: false,
                    summary: "social image aspect ratio is outside recommended range",
                },
                RuleDescriptor {
                    rule_id: "SOC011",
                    sitewide: false,
                    summary: "social image is larger than recommended",
                },
            ],
        },
        RuleGroupDefinition {
            name: "schema",
            title: "Structured Data",
            description: "",
            rules: &[
                RuleDescriptor {
                    rule_id: "SCH001",
                    sitewide: false,
                    summary: "invalid JSON-LD",
                },
                RuleDescriptor {
                    rule_id: "SCH002",
                    sitewide: false,
                    summary: "missing required schema type from config",
                },
                RuleDescriptor {
                    rule_id: "SCH003",
                    sitewide: false,
                    summary: "visible FAQ-like <details> blocks without FAQPage JSON-LD",
                },
                RuleDescriptor {
                    rule_id: "SCH004",
                    sitewide: false,
                    summary: "nested page missing BreadcrumbList JSON-LD when required",
                },
                RuleDescriptor {
                    rule_id: "SCH005",
                    sitewide: false,
                    summary: "JSON-LD name/headline does not align with the visible title/H1",
                },
                RuleDescriptor {
                    rule_id: "SCH006",
                    sitewide: false,
                    summary: "schema family object is missing required fields",
                },
                RuleDescriptor {
                    rule_id: "SCH007",
                    sitewide: false,
                    summary: "schema url does not align with canonical",
                },
                RuleDescriptor {
                    rule_id: "SCH008",
                    sitewide: false,
                    summary: "missing configured schema family",
                },
                RuleDescriptor {
                    rule_id: "SCH009",
                    sitewide: true,
                    summary: "sitewide schema entity graph is inconsistent",
                },
                RuleDescriptor {
                    rule_id: "SCH010",
                    sitewide: false,
                    summary: "docs-like page is missing docs-oriented schema",
                },
                RuleDescriptor {
                    rule_id: "SCH011",
                    sitewide: false,
                    summary: "home page is missing sitewide schema context",
                },
                RuleDescriptor {
                    rule_id: "SCH012",
                    sitewide: false,
                    summary: "listing-like page likely wants ItemList schema",
                },
                RuleDescriptor {
                    rule_id: "SCH013",
                    sitewide: false,
                    summary: "detail-like page looks under-described for its schema type",
                },
                RuleDescriptor {
                    rule_id: "SCH014",
                    sitewide: false,
                    summary: "docs-like page likely wants docs-oriented schema",
                },
                RuleDescriptor {
                    rule_id: "SCH015",
                    sitewide: false,
                    summary: "search page could expose SearchAction schema",
                },
                RuleDescriptor {
                    rule_id: "SCH016",
                    sitewide: false,
                    summary: "utility page should not repeat Organization schema inline",
                },
                RuleDescriptor {
                    rule_id: "SCH017",
                    sitewide: false,
                    summary: "editorial schema author is not visible on the page",
                },
                RuleDescriptor {
                    rule_id: "SCH018",
                    sitewide: false,
                    summary: "editorial schema dates are not visible on the page",
                },
            ],
        },
        RuleGroupDefinition {
            name: "llm",
            title: "LLM Artifacts",
            description: "",
            rules: &[
                RuleDescriptor {
                    rule_id: "LLM001",
                    sitewide: true,
                    summary: "missing llms.txt",
                },
                RuleDescriptor {
                    rule_id: "LLM002",
                    sitewide: true,
                    summary: "empty llms.txt",
                },
                RuleDescriptor {
                    rule_id: "LLM003",
                    sitewide: true,
                    summary: "missing expected page sections in llms.txt",
                },
                RuleDescriptor {
                    rule_id: "LLM004",
                    sitewide: true,
                    summary: "broken internal reference in llms.txt",
                },
                RuleDescriptor {
                    rule_id: "LLM005",
                    sitewide: true,
                    summary: "noncanonical .html links in llms.txt when extensionless canonicals are expected",
                },
                RuleDescriptor {
                    rule_id: "LLM006",
                    sitewide: true,
                    summary: "feature/category claim drift against feature-data.json",
                },
                RuleDescriptor {
                    rule_id: "LLM007",
                    sitewide: true,
                    summary: "feature-page count drift against feature-data.json",
                },
            ],
        },
        RuleGroupDefinition {
            name: "surfaces",
            title: "Machine Surfaces",
            description: "Machine-readable discovery, citation, and agent-retrieval surfaces.",
            rules: &[
                RuleDescriptor {
                    rule_id: "SRF001",
                    sitewide: true,
                    summary: "missing facts.json machine-readable facts manifest",
                },
                RuleDescriptor {
                    rule_id: "SRF002",
                    sitewide: true,
                    summary: "no per-page Markdown mirrors discovered",
                },
                RuleDescriptor {
                    rule_id: "SRF003",
                    sitewide: true,
                    summary: "larger site is missing llms-full.txt compiled context",
                },
                RuleDescriptor {
                    rule_id: "SRF004",
                    sitewide: true,
                    summary: "route has no discovered Markdown mirror",
                },
                RuleDescriptor {
                    rule_id: "SRF005",
                    sitewide: true,
                    summary: "route has Markdown mirror but no static discovery link",
                },
                RuleDescriptor {
                    rule_id: "SRF006",
                    sitewide: true,
                    summary: "llms.txt references missing machine-readable artifact",
                },
                RuleDescriptor {
                    rule_id: "SRF010",
                    sitewide: true,
                    summary: "agent-skills index missing despite tool-bearing manifest",
                },
                RuleDescriptor {
                    rule_id: "SRF011",
                    sitewide: true,
                    summary: "agent-skills index has invalid shape",
                },
                RuleDescriptor {
                    rule_id: "SRF015",
                    sitewide: true,
                    summary: "MCP server card missing despite MCP claim",
                },
                RuleDescriptor {
                    rule_id: "SRF016",
                    sitewide: true,
                    summary: "MCP server card has invalid shape",
                },
                RuleDescriptor {
                    rule_id: "SRF020",
                    sitewide: true,
                    summary: "API catalog missing despite API surface signal",
                },
                RuleDescriptor {
                    rule_id: "SRF021",
                    sitewide: true,
                    summary: "API catalog has invalid linkset shape",
                },
                RuleDescriptor {
                    rule_id: "SRF025",
                    sitewide: true,
                    summary: "OAuth-protected APIs missing OIDC/OAuth discovery metadata",
                },
                RuleDescriptor {
                    rule_id: "SRF026",
                    sitewide: true,
                    summary: "OAuth-protected APIs missing protected-resource metadata",
                },
                RuleDescriptor {
                    rule_id: "SRF030",
                    sitewide: true,
                    summary: "homepage doesn't honor `Accept: text/markdown` content negotiation",
                },
            ],
        },
        RuleGroupDefinition {
            name: "headers",
            title: "HTTP Response Headers",
            description: "Header-level rules that consult Page.response_headers (runtime audits) or do additional HTTP probes; silent on pure static audits.",
            rules: &[RuleDescriptor {
                rule_id: "LNK020",
                sitewide: true,
                summary: "homepage response sends no Link headers (RFC 8288)",
            }],
        },
        RuleGroupDefinition {
            name: "content",
            title: "Content Policy",
            description: "",
            rules: &[
                RuleDescriptor {
                    rule_id: "CNT001",
                    sitewide: false,
                    summary: "page is unusually small after stripping markup",
                },
                RuleDescriptor {
                    rule_id: "CNT002",
                    sitewide: false,
                    summary: "feature-like page is missing a configured section marker",
                },
                RuleDescriptor {
                    rule_id: "CNT003",
                    sitewide: false,
                    summary: "inline image is missing alt text",
                },
                RuleDescriptor {
                    rule_id: "CNT004",
                    sitewide: false,
                    summary: "inline image is too large",
                },
                RuleDescriptor {
                    rule_id: "CNT005",
                    sitewide: false,
                    summary: "duplicate visible content cluster",
                },
                RuleDescriptor {
                    rule_id: "CNT006",
                    sitewide: false,
                    summary: "generic-beneficiary copy without concrete anchors",
                },
            ],
        },
        RuleGroupDefinition {
            name: "editorial",
            title: "Editorial Policy",
            description: "Deterministic checks for route briefs that declare answer summaries, target questions, and claims requiring evidence. Findings are warnings by default and can be promoted by per-rule severity overrides.",
            rules: &[
                RuleDescriptor {
                    rule_id: "EDT001",
                    sitewide: true,
                    summary: "editorial route or declared answer summary is missing, misplaced, or outside the two-to-three sentence range",
                },
                RuleDescriptor {
                    rule_id: "EDT002",
                    sitewide: false,
                    summary: "declared target question is missing, mismatched, or has under 20 visible answer characters",
                },
                RuleDescriptor {
                    rule_id: "EDT003",
                    sitewide: false,
                    summary: "claim marked as requiring evidence has no usable evidence link",
                },
            ],
        },
        RuleGroupDefinition {
            name: "structure",
            title: "Retrieval Structure",
            description: "Reusable GEO rules extracted from the Chau7 website guidelines.",
            rules: &[
                RuleDescriptor {
                    rule_id: "GEO001",
                    sitewide: false,
                    summary: "<section> missing data-ui",
                },
                RuleDescriptor {
                    rule_id: "GEO002",
                    sitewide: false,
                    summary: "<article> missing data-ui",
                },
                RuleDescriptor {
                    rule_id: "GEO003",
                    sitewide: false,
                    summary: "duplicate data-ui on a page",
                },
                RuleDescriptor {
                    rule_id: "GEO004",
                    sitewide: false,
                    summary: "<section> missing a heading",
                },
                RuleDescriptor {
                    rule_id: "GEO005",
                    sitewide: false,
                    summary: "<details> missing <summary>",
                },
                RuleDescriptor {
                    rule_id: "GEO006",
                    sitewide: false,
                    summary: "<pre> missing nested <code>",
                },
                RuleDescriptor {
                    rule_id: "GEO007",
                    sitewide: false,
                    summary: "semantic block is too thin for retrieval",
                },
                RuleDescriptor {
                    rule_id: "GEO008",
                    sitewide: false,
                    summary: "page does not have enough answer-oriented blocks",
                },
                RuleDescriptor {
                    rule_id: "GEO009",
                    sitewide: false,
                    summary: "core page facts do not align across title, H1, OpenGraph, and schema",
                },
                RuleDescriptor {
                    rule_id: "GEO010",
                    sitewide: false,
                    summary: "numeric claims lack source cues",
                },
                RuleDescriptor {
                    rule_id: "GEO011",
                    sitewide: false,
                    summary: "page title is weakly disambiguated",
                },
                RuleDescriptor {
                    rule_id: "GEO012",
                    sitewide: false,
                    summary: "question-like block appears under-explained",
                },
                RuleDescriptor {
                    rule_id: "GEO013",
                    sitewide: false,
                    summary: "page contains overlapping answer chunks",
                },
            ],
        },
        RuleGroupDefinition {
            name: "runtime",
            title: "Runtime Crawl",
            description: "",
            rules: &[RuleDescriptor {
                rule_id: "CRW003",
                sitewide: true,
                summary: "crawl ended before the full internal route graph could be reviewed",
            }],
        },
        RuleGroupDefinition {
            name: "deployment",
            title: "Deployment Model",
            description: "",
            rules: &[RuleDescriptor {
                rule_id: "DEP001",
                sitewide: true,
                summary: "runtime deployment output detected; static directory audit may be incomplete",
            }],
        },
        RuleGroupDefinition {
            name: "agent_discovery",
            title: "Agent Discovery (AGT)",
            description: "Well-known machine-readable artifacts that signal a site's agent-facing capabilities. Off by default ([agent_discovery] enabled = true to opt in) because the underlying specs (RFC 9727 api-catalog, SEP-1649 MCP server-card) have nascent adoption. Use the post-crawl artifact probe (live `aexeo-cli crawl`) so HEAD-discoverable artifacts populate Site::indexed_paths automatically.",
            rules: &[
                RuleDescriptor {
                    rule_id: "AGT001",
                    sitewide: true,
                    summary: "missing /.well-known/api-catalog (RFC 9727)",
                },
                RuleDescriptor {
                    rule_id: "AGT002",
                    sitewide: true,
                    summary: "missing /.well-known/mcp/server-card.json (SEP-1649)",
                },
            ],
        },
        RuleGroupDefinition {
            name: "accessibility",
            title: "Accessibility (A11Y)",
            description: "Static accessibility checks. Catches the deterministic HTML-spec violations that affect screen-reader users (missing alt, empty buttons, duplicate ids, heading jumps, missing landmarks, placeholder alt text). Browser-backed checks (focus order, contrast, ARIA semantics) are out of scope for the static auditor.",
            rules: &[
                RuleDescriptor {
                    rule_id: "A11Y001",
                    sitewide: false,
                    summary: "<img> missing alt attribute (skipped on canonically decorative images in default mode)",
                },
                RuleDescriptor {
                    rule_id: "A11Y002",
                    sitewide: false,
                    summary: "<a> or <button> with no accessible text or label",
                },
                RuleDescriptor {
                    rule_id: "A11Y003",
                    sitewide: false,
                    summary: "duplicate id attribute on the same page",
                },
                RuleDescriptor {
                    rule_id: "A11Y004",
                    sitewide: false,
                    summary: "heading hierarchy jumps a level (e.g. h2 → h4)",
                },
                RuleDescriptor {
                    rule_id: "A11Y005",
                    sitewide: false,
                    summary: "page has no <main> landmark or role=\"main\" element",
                },
                RuleDescriptor {
                    rule_id: "A11Y006",
                    sitewide: false,
                    summary: "alt text matches the image filename (likely placeholder)",
                },
            ],
        },
        RuleGroupDefinition {
            name: "quality",
            title: "Internal Quality",
            description: "Repository self-checks for the Aexeo workspace itself. These audit the project rather than a reviewed site: required documentation, generated-doc drift, workspace wiring, and source hygiene such as forbidden macros, unwrap/expect in production code, and unsafe markers. Enabled by the `quality` command, not by `check`.",
            rules: &[
                RuleDescriptor {
                    rule_id: "QLT004",
                    sitewide: true,
                    summary: "missing required project documentation file",
                },
                RuleDescriptor {
                    rule_id: "QLT005",
                    sitewide: true,
                    summary: "built-in rule group missing from docs/rules.md",
                },
                RuleDescriptor {
                    rule_id: "QLT007",
                    sitewide: true,
                    summary: "generated docs drift from code and must be regenerated",
                },
                RuleDescriptor {
                    rule_id: "QLT009",
                    sitewide: true,
                    summary: "missing Cargo workspace manifest",
                },
                RuleDescriptor {
                    rule_id: "QLT010",
                    sitewide: true,
                    summary: "missing Rust build script",
                },
                RuleDescriptor {
                    rule_id: "QLT011",
                    sitewide: true,
                    summary: "missing performance budget file",
                },
                RuleDescriptor {
                    rule_id: "QLT012",
                    sitewide: true,
                    summary: "missing Rust CLI integration test coverage",
                },
                RuleDescriptor {
                    rule_id: "QLT013",
                    sitewide: true,
                    summary: "missing install script",
                },
                RuleDescriptor {
                    rule_id: "QLT014",
                    sitewide: true,
                    summary: "missing local CI script",
                },
                RuleDescriptor {
                    rule_id: "QLT015",
                    sitewide: true,
                    summary: "missing git hook installation script",
                },
                RuleDescriptor {
                    rule_id: "QLT016",
                    sitewide: true,
                    summary: "missing pre-commit hook",
                },
                RuleDescriptor {
                    rule_id: "QLT017",
                    sitewide: true,
                    summary: "missing pre-push hook",
                },
                RuleDescriptor {
                    rule_id: "QLT018",
                    sitewide: true,
                    summary: "debug or placeholder macro in non-test Rust source",
                },
                RuleDescriptor {
                    rule_id: "QLT019",
                    sitewide: true,
                    summary: "unwrap or expect in non-test Rust source",
                },
                RuleDescriptor {
                    rule_id: "QLT020",
                    sitewide: true,
                    summary: "unsafe Rust marker in non-test Rust source",
                },
                RuleDescriptor {
                    rule_id: "QLT021",
                    sitewide: true,
                    summary: "missing cargo-deny policy file",
                },
                RuleDescriptor {
                    rule_id: "QLT022",
                    sitewide: true,
                    summary: "missing dependency hygiene script",
                },
                RuleDescriptor {
                    rule_id: "QLT023",
                    sitewide: true,
                    summary: "missing Node package lockfile for browser runtime",
                },
                RuleDescriptor {
                    rule_id: "QLT024",
                    sitewide: true,
                    summary: "config key declared and documented but never read",
                },
            ],
        },
    ]
}

/// Human-readable group title for `rule_id`, e.g. `"A11Y001" -> "Accessibility (A11Y)"`.
///
/// Resolved from the registry rather than a second prefix table so a group
/// added above is picked up everywhere automatically. Returns `None` for ids
/// the registry does not know.
pub fn rule_group_title_for_id(rule_id: &str) -> Option<&'static str> {
    builtin_rule_groups()
        .iter()
        .find(|group| group.rules.iter().any(|rule| rule.rule_id == rule_id))
        .map(|group| group.title)
}

pub fn builtin_adapters() -> &'static [AdapterDefinition] {
    &[
        AdapterDefinition {
            name: "nextjs-export",
            description: "Use static export output from Next.js projects, typically ./out.",
            priority: 30,
        },
        AdapterDefinition {
            name: "astro-dist",
            description: "Use generated Astro static output, typically ./dist.",
            priority: 20,
        },
        AdapterDefinition {
            name: "docusaurus-build",
            description: "Use generated Docusaurus output, typically ./build.",
            priority: 10,
        },
        AdapterDefinition {
            name: "generic",
            description: "Use the provided path directly, or source_dir when configured.",
            priority: 0,
        },
    ]
}

pub fn list_rule_group_names() -> Vec<&'static str> {
    builtin_rule_groups()
        .iter()
        .map(|group| group.name)
        .collect()
}

pub fn list_adapter_names() -> Vec<&'static str> {
    builtin_adapters()
        .iter()
        .map(|adapter| adapter.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        assert_exhaustive_layer, builtin_rule_groups, is_known_layer_prefix,
        is_known_metadata_prefix, layers_for_prefix, list_adapter_names, list_rule_group_names,
        rule_group_title_for_id, rule_layers_for_id, rule_metadata_for_id, rule_prefix,
    };
    use aexeo_contracts::{ConfidenceLevel, Layer, RuleClass};

    #[test]
    fn preserves_rule_group_order() {
        assert_eq!(
            list_rule_group_names(),
            vec![
                "html",
                "links",
                "sitemap",
                "robots",
                "social",
                "schema",
                "llm",
                "surfaces",
                "headers",
                "content",
                "editorial",
                "structure",
                "runtime",
                "deployment",
                "agent_discovery",
                "accessibility",
                "quality",
            ]
        );
    }

    #[test]
    fn preserves_adapter_order() {
        assert_eq!(
            list_adapter_names(),
            vec!["nextjs-export", "astro-dist", "docusaurus-build", "generic"]
        );
    }

    /// Regression: every alphanumeric family must resolve to a real group
    /// title. `reporting::rule_group_name` used a `take_while(uppercase)`
    /// prefix scan with no arms for these prefixes, so all of them rendered
    /// as "Other" in report headings, the section recap, and the Search
    /// Console CSV export.
    ///
    /// `FACTS` is deliberately absent: it survives only as a layer mapping and
    /// a doc comment, with no live rules behind it.
    #[test]
    fn every_alphanumeric_rule_family_resolves_to_a_group_title() {
        for id in ["A11Y001", "SRF001", "AGT001", "EDT001", "SEO001"] {
            assert!(
                rule_group_title_for_id(id).is_some(),
                "{id} should resolve to a registered group title"
            );
        }
        assert_eq!(
            rule_group_title_for_id("A11Y001"),
            Some("Accessibility (A11Y)")
        );
        assert_eq!(
            rule_group_title_for_id("AGT001"),
            Some("Agent Discovery (AGT)")
        );
        assert_eq!(rule_group_title_for_id("QLT004"), Some("Internal Quality"));
    }

    /// Every registered rule must belong to exactly one group, and every
    /// group must have a non-empty title. `docs/rules.md` is generated from
    /// this list, so an unregistered rule silently disappears from the
    /// published inventory.
    #[test]
    fn every_registered_rule_appears_exactly_once_across_groups() {
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for group in builtin_rule_groups() {
            assert!(!group.title.is_empty(), "{} has no title", group.name);
            for rule in group.rules {
                assert!(
                    !rule.summary.is_empty(),
                    "{} has no summary for {}",
                    group.name,
                    rule.rule_id
                );
                seen.push((rule.rule_id, group.name));
            }
        }
        let mut ids: Vec<&str> = seen.iter().map(|(id, _)| *id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "a rule id is registered in two groups");
        assert!(!ids.is_empty());
    }

    #[test]
    fn unknown_rule_ids_have_no_group_title() {
        assert_eq!(rule_group_title_for_id("ZZZ999"), None);
        assert_eq!(rule_group_title_for_id(""), None);
    }

    /// Guards the invariant the reporting layer now depends on: every
    /// registered rule carries scope metadata, and the sitewide set matches
    /// the groups that emit `FindingScope::Sitewide`.
    #[test]
    fn sitewide_scope_is_recorded_for_the_expected_families() {
        let find = |id: &str| {
            builtin_rule_groups()
                .iter()
                .flat_map(|group| group.rules.iter())
                .find(|rule| rule.rule_id == id)
                .map(|rule| rule.sitewide)
        };
        for id in [
            "MAP001", "ROB001", "LLM001", "DEP001", "QLT004", "AGT001", "SRF001",
        ] {
            assert_eq!(find(id), Some(true), "{id} should be sitewide");
        }
        for id in ["SEO011", "SEO017", "LNK020", "SCH009", "EDT001", "CRW003"] {
            assert_eq!(find(id), Some(true), "{id} should be sitewide");
        }
        for id in ["SEO001", "LNK001", "A11Y001", "SCH001", "CNT001", "EDT002"] {
            assert_eq!(find(id), Some(false), "{id} should be page-scoped");
        }
    }

    #[test]
    fn exposes_rule_metadata() {
        let seo = rule_metadata_for_id("SEO001");
        assert!(matches!(seo.class, RuleClass::Hard));
        assert!(matches!(seo.confidence, ConfidenceLevel::High));

        let geo = rule_metadata_for_id("GEO007");
        assert!(matches!(geo.class, RuleClass::Heuristic));
    }

    #[test]
    fn rule_prefix_strips_trailing_digits() {
        // The prior take_while(uppercase) form yielded "A" for "A11Y001"
        // and silently fell through to the default citability layer,
        // losing the accessibility mapping. Lock that in.
        assert_eq!(rule_prefix("SEO001"), "SEO");
        assert_eq!(rule_prefix("FACTS003"), "FACTS");
        assert_eq!(rule_prefix("A11Y001"), "A11Y");
        assert_eq!(rule_prefix("A11Y042"), "A11Y");
        // No trailing digits is fine — returns the whole id.
        assert_eq!(rule_prefix("UNKNOWN"), "UNKNOWN");
    }

    #[test]
    fn a11y_prefix_maps_to_accessibility_layer() {
        // A11Y003 is the cleanest test of the prefix-only default
        // since it's primary-only on Accessibility — no per-rule
        // override touches it. (Other A11Y rules carry GEO secondaries
        // for cross-tag impact; see the per_rule_a11y_overrides test.)
        let layers = rule_layers_for_id("A11Y003");
        assert_eq!(layers.primary, Layer::Accessibility);
        assert!(layers.secondaries.is_empty());

        let metadata = rule_metadata_for_id("A11Y001");
        assert!(matches!(metadata.class, RuleClass::Hard));
        assert!(matches!(metadata.confidence, ConfidenceLevel::High));
    }

    #[test]
    fn per_rule_a11y_overrides_carry_geo_secondaries() {
        // A11Y001 / A11Y006: alt text → image search and crawlers,
        // so accessibility primary + retrievability secondary.
        for rule_id in ["A11Y001", "A11Y006"] {
            let layers = rule_layers_for_id(rule_id);
            assert_eq!(layers.primary, Layer::Accessibility, "{}", rule_id);
            assert!(
                layers.secondaries.contains(&Layer::Retrievability),
                "{} should carry Retrievability secondary",
                rule_id
            );
        }
        // A11Y002 / A11Y004 / A11Y005: link graph, heading shape,
        // landmarks all feed citability.
        for rule_id in ["A11Y002", "A11Y004", "A11Y005"] {
            let layers = rule_layers_for_id(rule_id);
            assert_eq!(layers.primary, Layer::Accessibility, "{}", rule_id);
            assert!(
                layers.secondaries.contains(&Layer::Citability),
                "{} should carry Citability secondary",
                rule_id
            );
        }
    }

    #[test]
    fn assigns_layers_by_prefix_default() {
        // SEO defaults to citability primary, retrievability secondary.
        let seo = rule_layers_for_id("SEO001");
        assert_eq!(seo.primary, Layer::Citability);
        assert!(seo.secondaries.contains(&Layer::Retrievability));

        // ROB and MAP are pure retrievability.
        assert_eq!(rule_layers_for_id("ROB001").primary, Layer::Retrievability);
        assert!(rule_layers_for_id("ROB001").secondaries.is_empty());
        assert_eq!(rule_layers_for_id("MAP001").primary, Layer::Retrievability);

        // SCH defaults to citability.
        assert_eq!(rule_layers_for_id("SCH001").primary, Layer::Citability);
        assert!(
            rule_layers_for_id("SCH001")
                .secondaries
                .contains(&Layer::Absorbability)
        );

        // LLM defaults to absorbability.
        assert_eq!(rule_layers_for_id("LLM001").primary, Layer::Absorbability);
    }

    #[test]
    fn applies_per_rule_layer_overrides() {
        // SCH011 / SCH015 are retrievability-primary, not citability.
        assert_eq!(
            rule_layers_for_id("SCH011").primary,
            Layer::Retrievability,
            "SCH011 (home sitewide context) is retrievability-primary"
        );
        assert_eq!(
            rule_layers_for_id("SCH015").primary,
            Layer::Retrievability,
            "SCH015 (search SearchAction) is retrievability-primary"
        );
        // SRF005 / SRF006 are absorbability-primary (mirror discoverability
        // is about reaching content the generator absorbs).
        assert_eq!(
            rule_layers_for_id("SRF005").primary,
            Layer::Absorbability,
            "SRF005 mirror-discoverability is absorbability-primary"
        );
    }

    #[test]
    fn every_rule_in_registry_has_a_layer_assignment() {
        // Two things are checked here, and the original version of this test
        // only really managed one of them.
        //
        // The `match` on `Layer` is a compile-time exhaustiveness guard:
        // adding a variant breaks the build rather than letting a rule
        // silently fall through to a default. That part was real.
        //
        // It was also the *only* part, because every arm of that match
        // returned `true`, so the runtime `assert!` could never fail. What
        // it could not catch was a rule whose prefix had no entry in the
        // table and therefore got the fallback — which is exactly the bug
        // that made `A11Y`, `SRF`, `AGT`, and `EDT` findings report as
        // "Other" until the group lookup replaced the prefix table.
        //
        // The runtime check is now on the thing that can actually regress:
        // that the rule's own prefix is a registered family. `match layers.primary`
        // is kept only as the exhaustiveness guard it genuinely is.
        for group in builtin_rule_groups() {
            for descriptor in group.rules {
                assert!(
                    is_known_layer_prefix(rule_prefix(descriptor.rule_id)),
                    "rule {} has no layer mapping for prefix {}",
                    descriptor.rule_id,
                    rule_prefix(descriptor.rule_id)
                );
                let layers = rule_layers_for_id(descriptor.rule_id);
                // Compile-time exhaustiveness guard: adding a `Layer` variant
                // breaks this build rather than letting a rule reach a
                // default arm silently. Kept as a bare expression because
                // binding a unit value trips clippy's `let_unit_value`.
                assert_exhaustive_layer(layers.primary);
            }
        }
    }

    /// Same idea for the metadata table: an unregistered family must be a
    /// loud gap rather than a silent default.
    #[test]
    fn every_registered_prefix_has_explicit_metadata() {
        for group in builtin_rule_groups() {
            for descriptor in group.rules {
                let prefix = rule_prefix(descriptor.rule_id);
                assert!(
                    is_known_metadata_prefix(prefix),
                    "rule {} has no metadata mapping for prefix {prefix}",
                    descriptor.rule_id
                );
            }
        }
    }

    /// The families that were once missing from the layer table, pinned
    /// explicitly so a future refactor cannot quietly drop them again.
    #[test]
    fn formerly_missing_families_have_layer_mappings() {
        for prefix in ["A11Y", "SRF", "AGT", "EDT", "SCH", "SEO", "LLM", "QLT"] {
            assert!(
                is_known_layer_prefix(prefix),
                "{prefix} should have an explicit layer mapping"
            );
        }
        assert_eq!(layers_for_prefix("A11Y").primary, Layer::Accessibility);
        assert_eq!(layers_for_prefix("AGT").primary, Layer::Retrievability);
    }

    #[test]
    fn unknown_prefixes_fall_back_to_citability() {
        // The fallback still has to exist for robustness; these tests exist
        // so that "unknown" cannot quietly become a registered family.
        assert!(!is_known_layer_prefix("ZZZZ"));
        assert_eq!(layers_for_prefix("ZZZZ").primary, Layer::Citability);
    }
}
