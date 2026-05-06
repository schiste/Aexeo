use aexeo_contracts::{ConfidenceLevel, Layer, RuleClass, RuleLayers, RuleMetadata};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDescriptor {
    pub rule_id: &'static str,
    pub summary: &'static str,
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

fn metadata_for_prefix(prefix: &str) -> RuleMetadata {
    match prefix {
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
        _ => RuleMetadata {
            class: RuleClass::Heuristic,
            confidence: ConfidenceLevel::Medium,
        },
    }
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

/// Default layer assignment by rule-ID prefix. The prefix usually
/// determines the family (LNK = retrievability links, SCH = citability
/// schema, etc.); per-rule overrides in `rule_layers_for_id` handle the
/// outliers.
fn layers_for_prefix(prefix: &str) -> RuleLayers {
    match prefix {
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
        // Unknown prefix: default to citability (most rules are about
        // making the page worth citing). Better than crashing.
        _ => RuleLayers::primary_only(Layer::Citability),
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
                    summary: "missing <title>",
                },
                RuleDescriptor {
                    rule_id: "SEO002",
                    summary: "missing meta description",
                },
                RuleDescriptor {
                    rule_id: "SEO004",
                    summary: "missing canonical",
                },
                RuleDescriptor {
                    rule_id: "SEO005",
                    summary: "missing <h1>",
                },
                RuleDescriptor {
                    rule_id: "SEO006",
                    summary: "multiple <h1> tags",
                },
                RuleDescriptor {
                    rule_id: "SEO007",
                    summary: "missing root html lang attribute",
                },
                RuleDescriptor {
                    rule_id: "SEO008",
                    summary: "page has hreflang alternates but no self-referencing hreflang",
                },
                RuleDescriptor {
                    rule_id: "SEO009",
                    summary: "hreflang alternate points to a missing internal path",
                },
                RuleDescriptor {
                    rule_id: "SEO010",
                    summary: "invalid hreflang locale code",
                },
                RuleDescriptor {
                    rule_id: "SEO011",
                    summary: "hreflang cluster is missing x-default",
                },
                RuleDescriptor {
                    rule_id: "SEO012",
                    summary: "hreflang alternate is not reciprocally linked",
                },
                RuleDescriptor {
                    rule_id: "SEO013",
                    summary: "page suppresses snippets via nosnippet",
                },
                RuleDescriptor {
                    rule_id: "SEO014",
                    summary: "page restricts snippets via max-snippet",
                },
                RuleDescriptor {
                    rule_id: "SEO015",
                    summary: "page uses data-nosnippet blocks",
                },
                RuleDescriptor {
                    rule_id: "SEO016",
                    summary: "page canonicals to another crawlable route",
                },
                RuleDescriptor {
                    rule_id: "SEO017",
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
                    summary: "broken internal link",
                },
                RuleDescriptor {
                    rule_id: "LNK002",
                    summary: "orphan page",
                },
                RuleDescriptor {
                    rule_id: "LNK003",
                    summary: "weak internal anchor text",
                },
                RuleDescriptor {
                    rule_id: "LNK004",
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
                    summary: "missing sitemap.xml",
                },
                RuleDescriptor {
                    rule_id: "MAP002",
                    summary: "invalid sitemap XML",
                },
                RuleDescriptor {
                    rule_id: "MAP003",
                    summary: "empty sitemap set",
                },
                RuleDescriptor {
                    rule_id: "MAP004",
                    summary: "canonical missing from sitemap coverage",
                },
                RuleDescriptor {
                    rule_id: "MAP005",
                    summary: "sitemap.xml exists but is empty",
                },
                RuleDescriptor {
                    rule_id: "MAP006",
                    summary: "sitemap endpoint returned HTML instead of sitemap XML",
                },
                RuleDescriptor {
                    rule_id: "MAP007",
                    summary: "sitemap endpoint exists but is not recognizable sitemap XML",
                },
                RuleDescriptor {
                    rule_id: "MAP008",
                    summary: "sitemap.xml is missing lastmod values",
                },
                RuleDescriptor {
                    rule_id: "MAP009",
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
                    summary: "missing robots.txt",
                },
                RuleDescriptor {
                    rule_id: "ROB002",
                    summary: "missing Sitemap: declaration in robots.txt",
                },
                RuleDescriptor {
                    rule_id: "ROB003",
                    summary: "robots.txt blocks the whole site for User-agent: *",
                },
                RuleDescriptor {
                    rule_id: "ROB004",
                    summary: "page is in sitemap but declares noindex in meta robots",
                },
                RuleDescriptor {
                    rule_id: "ROB005",
                    summary: "page declares both canonical and noindex",
                },
                RuleDescriptor {
                    rule_id: "ROB006",
                    summary: "page declares nofollow",
                },
                RuleDescriptor {
                    rule_id: "ROB007",
                    summary: "robots.txt may overblock crawl budget",
                },
                RuleDescriptor {
                    rule_id: "ROB008",
                    summary: "page is in sitemap but declares noindex in X-Robots-Tag",
                },
                RuleDescriptor {
                    rule_id: "ROB010",
                    summary: "robots.txt has no AI-bot User-agent directives",
                },
                RuleDescriptor {
                    rule_id: "ROB011",
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
                    summary: "missing og:title",
                },
                RuleDescriptor {
                    rule_id: "SOC002",
                    summary: "missing og:description",
                },
                RuleDescriptor {
                    rule_id: "SOC003",
                    summary: "missing og:type",
                },
                RuleDescriptor {
                    rule_id: "SOC004",
                    summary: "missing twitter:card",
                },
                RuleDescriptor {
                    rule_id: "SOC005",
                    summary: "og:url does not match canonical",
                },
                RuleDescriptor {
                    rule_id: "SOC006",
                    summary: "missing og:image",
                },
                RuleDescriptor {
                    rule_id: "SOC007",
                    summary: "missing twitter:image",
                },
                RuleDescriptor {
                    rule_id: "SOC008",
                    summary: "social image points to a missing internal asset",
                },
                RuleDescriptor {
                    rule_id: "SOC009",
                    summary: "twitter:card is `summary`; consider `summary_large_image`",
                },
                RuleDescriptor {
                    rule_id: "SOC010",
                    summary: "social image aspect ratio is outside recommended range",
                },
                RuleDescriptor {
                    rule_id: "SOC011",
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
                    summary: "invalid JSON-LD",
                },
                RuleDescriptor {
                    rule_id: "SCH002",
                    summary: "missing required schema type from config",
                },
                RuleDescriptor {
                    rule_id: "SCH003",
                    summary: "visible FAQ-like <details> blocks without FAQPage JSON-LD",
                },
                RuleDescriptor {
                    rule_id: "SCH004",
                    summary: "nested page missing BreadcrumbList JSON-LD when required",
                },
                RuleDescriptor {
                    rule_id: "SCH005",
                    summary: "JSON-LD name/headline does not align with the visible title/H1",
                },
                RuleDescriptor {
                    rule_id: "SCH006",
                    summary: "schema family object is missing required fields",
                },
                RuleDescriptor {
                    rule_id: "SCH007",
                    summary: "schema url does not align with canonical",
                },
                RuleDescriptor {
                    rule_id: "SCH008",
                    summary: "missing configured schema family",
                },
                RuleDescriptor {
                    rule_id: "SCH009",
                    summary: "sitewide schema entity graph is inconsistent",
                },
                RuleDescriptor {
                    rule_id: "SCH010",
                    summary: "docs-like page is missing docs-oriented schema",
                },
                RuleDescriptor {
                    rule_id: "SCH011",
                    summary: "home page is missing sitewide schema context",
                },
                RuleDescriptor {
                    rule_id: "SCH012",
                    summary: "listing-like page likely wants ItemList schema",
                },
                RuleDescriptor {
                    rule_id: "SCH013",
                    summary: "detail-like page looks under-described for its schema type",
                },
                RuleDescriptor {
                    rule_id: "SCH014",
                    summary: "docs-like page likely wants docs-oriented schema",
                },
                RuleDescriptor {
                    rule_id: "SCH015",
                    summary: "search page could expose SearchAction schema",
                },
                RuleDescriptor {
                    rule_id: "SCH016",
                    summary: "utility page should not repeat Organization schema inline",
                },
                RuleDescriptor {
                    rule_id: "SCH017",
                    summary: "editorial schema author is not visible on the page",
                },
                RuleDescriptor {
                    rule_id: "SCH018",
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
                    summary: "missing llms.txt",
                },
                RuleDescriptor {
                    rule_id: "LLM002",
                    summary: "empty llms.txt",
                },
                RuleDescriptor {
                    rule_id: "LLM003",
                    summary: "missing expected page sections in llms.txt",
                },
                RuleDescriptor {
                    rule_id: "LLM004",
                    summary: "broken internal reference in llms.txt",
                },
                RuleDescriptor {
                    rule_id: "LLM005",
                    summary: "noncanonical .html links in llms.txt when extensionless canonicals are expected",
                },
                RuleDescriptor {
                    rule_id: "LLM006",
                    summary: "feature/category claim drift against feature-data.json",
                },
                RuleDescriptor {
                    rule_id: "LLM007",
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
                    summary: "missing facts.json machine-readable facts manifest",
                },
                RuleDescriptor {
                    rule_id: "SRF002",
                    summary: "no per-page Markdown mirrors discovered",
                },
                RuleDescriptor {
                    rule_id: "SRF003",
                    summary: "larger site is missing llms-full.txt compiled context",
                },
                RuleDescriptor {
                    rule_id: "SRF004",
                    summary: "route has no discovered Markdown mirror",
                },
                RuleDescriptor {
                    rule_id: "SRF005",
                    summary: "route has Markdown mirror but no static discovery link",
                },
                RuleDescriptor {
                    rule_id: "SRF006",
                    summary: "llms.txt references missing machine-readable artifact",
                },
                RuleDescriptor {
                    rule_id: "SRF010",
                    summary: "agent-skills index missing despite tool-bearing manifest",
                },
                RuleDescriptor {
                    rule_id: "SRF011",
                    summary: "agent-skills index has invalid shape",
                },
                RuleDescriptor {
                    rule_id: "SRF015",
                    summary: "MCP server card missing despite MCP claim",
                },
                RuleDescriptor {
                    rule_id: "SRF016",
                    summary: "MCP server card has invalid shape",
                },
                RuleDescriptor {
                    rule_id: "SRF020",
                    summary: "API catalog missing despite API surface signal",
                },
                RuleDescriptor {
                    rule_id: "SRF021",
                    summary: "API catalog has invalid linkset shape",
                },
                RuleDescriptor {
                    rule_id: "SRF025",
                    summary: "OAuth-protected APIs missing OIDC/OAuth discovery metadata",
                },
                RuleDescriptor {
                    rule_id: "SRF026",
                    summary: "OAuth-protected APIs missing protected-resource metadata",
                },
                RuleDescriptor {
                    rule_id: "SRF030",
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
                    summary: "page is unusually small after stripping markup",
                },
                RuleDescriptor {
                    rule_id: "CNT002",
                    summary: "feature-like page is missing a configured section marker",
                },
                RuleDescriptor {
                    rule_id: "CNT003",
                    summary: "inline image is missing alt text",
                },
                RuleDescriptor {
                    rule_id: "CNT004",
                    summary: "inline image is too large",
                },
                RuleDescriptor {
                    rule_id: "CNT005",
                    summary: "duplicate visible content cluster",
                },
                RuleDescriptor {
                    rule_id: "CNT006",
                    summary: "generic-beneficiary copy without concrete anchors",
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
                    summary: "<section> missing data-ui",
                },
                RuleDescriptor {
                    rule_id: "GEO002",
                    summary: "<article> missing data-ui",
                },
                RuleDescriptor {
                    rule_id: "GEO003",
                    summary: "duplicate data-ui on a page",
                },
                RuleDescriptor {
                    rule_id: "GEO004",
                    summary: "<section> missing a heading",
                },
                RuleDescriptor {
                    rule_id: "GEO005",
                    summary: "<details> missing <summary>",
                },
                RuleDescriptor {
                    rule_id: "GEO006",
                    summary: "<pre> missing nested <code>",
                },
                RuleDescriptor {
                    rule_id: "GEO007",
                    summary: "semantic block is too thin for retrieval",
                },
                RuleDescriptor {
                    rule_id: "GEO008",
                    summary: "page does not have enough answer-oriented blocks",
                },
                RuleDescriptor {
                    rule_id: "GEO009",
                    summary: "core page facts do not align across title, H1, OpenGraph, and schema",
                },
                RuleDescriptor {
                    rule_id: "GEO010",
                    summary: "numeric claims lack source cues",
                },
                RuleDescriptor {
                    rule_id: "GEO011",
                    summary: "page title is weakly disambiguated",
                },
                RuleDescriptor {
                    rule_id: "GEO012",
                    summary: "question-like block appears under-explained",
                },
                RuleDescriptor {
                    rule_id: "GEO013",
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
                summary: "crawl ended before the full internal route graph could be reviewed",
            }],
        },
        RuleGroupDefinition {
            name: "deployment",
            title: "Deployment Model",
            description: "",
            rules: &[RuleDescriptor {
                rule_id: "DEP001",
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
                    summary: "missing /.well-known/api-catalog (RFC 9727)",
                },
                RuleDescriptor {
                    rule_id: "AGT002",
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
                    summary: "<img> missing alt attribute (skipped on canonically decorative images in default mode)",
                },
                RuleDescriptor {
                    rule_id: "A11Y002",
                    summary: "<a> or <button> with no accessible text or label",
                },
                RuleDescriptor {
                    rule_id: "A11Y003",
                    summary: "duplicate id attribute on the same page",
                },
                RuleDescriptor {
                    rule_id: "A11Y004",
                    summary: "heading hierarchy jumps a level (e.g. h2 → h4)",
                },
                RuleDescriptor {
                    rule_id: "A11Y005",
                    summary: "page has no <main> landmark or role=\"main\" element",
                },
                RuleDescriptor {
                    rule_id: "A11Y006",
                    summary: "alt text matches the image filename (likely placeholder)",
                },
            ],
        },
    ]
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
        builtin_rule_groups, list_adapter_names, list_rule_group_names, rule_layers_for_id,
        rule_metadata_for_id, rule_prefix,
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
                "structure",
                "runtime",
                "deployment",
                "agent_discovery",
                "accessibility",
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
        // Smoke test: layer_for_id must not panic on any registered rule
        // and every assignment must have a sensible primary layer.
        // The match expression below relies on Layer's exhaustiveness —
        // adding a new Layer variant deliberately fails compile here so
        // we don't silently miss it.
        for group in builtin_rule_groups() {
            for descriptor in group.rules {
                let layers = rule_layers_for_id(descriptor.rule_id);
                let valid = match layers.primary {
                    Layer::Retrievability
                    | Layer::Citability
                    | Layer::Absorbability
                    | Layer::EntityLegitimacy
                    | Layer::Accessibility => true,
                };
                assert!(
                    valid,
                    "rule {} has no valid primary layer",
                    descriptor.rule_id
                );
            }
        }
    }
}
