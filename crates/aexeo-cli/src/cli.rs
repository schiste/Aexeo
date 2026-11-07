use aexeo_core::intelligence::presence::{DEFAULT_WIKIPEDIA_LANGUAGE, is_wikipedia_language};
use anyhow::{Context, Result};
use clap::{Arg, ArgAction, Command, value_parser};

pub fn build_cli() -> Command {
    Command::new("aexeo-cli")
        .about("SEO and GEO linting for static websites")
        .subcommand(
            Command::new("link-graph")
                .about("Analyze explicitly identified rendered pages and inventory")
                .arg(Arg::new("input").required(true).help("Versioned JSON input file"))
                .arg(Arg::new("audit").long("audit").action(ArgAction::SetTrue).help("Read an Aexeo audit artifact instead of a link graph input"))
                .arg(Arg::new("context").long("context").requires("audit").num_args(1).help("JSON audit identity and inventory context"))
                .arg(Arg::new("static-url-map").long("static-url-map").requires("audit").num_args(1).help("JSON map of static relative paths to public URLs"))
                .arg(Arg::new("telemetry").long("telemetry").action(ArgAction::SetTrue).help("Emit aggregate timings and counts as JSON on stderr")),
        )
        .subcommand(
            Command::new("feedback-export")
                .about("Export validated human feedback as a historical review event")
                .arg(Arg::new("corpus").long("corpus").required(true))
                .arg(Arg::new("note").long("note").required(true))
                .arg(Arg::new("output").long("output").required(true))
                .arg(Arg::new("telemetry").long("telemetry").action(ArgAction::SetTrue)),
        )
        .subcommand(
            Command::new("feedback-capture")
                .about("Capture explicitly reported human feedback in a private local note")
                .arg(Arg::new("text").long("text").required(true))
                .arg(Arg::new("report").long("report").required(true))
                .arg(Arg::new("gate").long("gate").required(true))
                .arg(Arg::new("example").long("example").required(true))
                .arg(Arg::new("human-status").long("human-status").value_parser(["passed", "failed", "uncertain"]))
                .arg(Arg::new("reviewer").long("reviewer"))
                .arg(Arg::new("method").long("method"))
                .arg(Arg::new("reason-file").long("reason-file"))
                .arg(Arg::new("language").long("language"))
                .arg(Arg::new("content-type").long("content-type"))
                .arg(Arg::new("preset").long("preset"))
                .arg(Arg::new("config-version").long("config-version"))
                .arg(Arg::new("tool-version").long("tool-version"))
                .arg(Arg::new("output").long("output").required(true))
                .arg(Arg::new("telemetry").long("telemetry").action(ArgAction::SetTrue)),
        )
        .subcommand(
            Command::new("content")
                .about("Run a portable content preset on a UTF-8 file or stdin")
                .arg(Arg::new("path").default_value("-"))
                .arg(Arg::new("preset").long("preset").default_value("article"))
                .arg(Arg::new("preset-source").long("preset-source").num_args(1).action(ArgAction::Append).help("Load a named TOML preset as NAME=PATH; may be repeated"))
                .arg(Arg::new("metadata").long("metadata").num_args(1).help("JSON file containing string metadata fields"))
                .arg(Arg::new("evidence-source").long("evidence-source").num_args(1).action(ArgAction::Append).conflicts_with("crawl-url").help("Load a named UTF-8 source snapshot as NAME=PATH; may be repeated"))
                .arg(Arg::new("site").long("site").num_args(1).help("Load a static website for site gates"))
                .arg(Arg::new("collection").long("collection").num_args(1).conflicts_with("crawl-url").help("Load a complete JSON array of content items for collection gates"))
                .arg(Arg::new("site-config").long("site-config").num_args(1).requires("site").help("Existing site audit config file"))
                .arg(Arg::new("crawl-url").long("crawl-url").num_args(1).conflicts_with_all(["site", "fix", "metadata"]).help("Crawl a website and evaluate the preset against its snapshot"))
                .arg(Arg::new("crawl-config").long("crawl-config").num_args(1).requires("crawl-url").help("Existing crawl configuration file"))
                .arg(Arg::new("crawl-max-pages").long("crawl-max-pages").value_parser(value_parser!(usize)).default_value("200").help("Maximum pages to crawl"))
                .arg(Arg::new("crawl-engine").long("crawl-engine").value_parser(["http", "playwright"]).default_value("http").help("Crawl engine"))
                .arg(Arg::new("input-format").long("input-format").value_parser(["text", "markdown"]).default_value("text").help("Interpret content as raw text or Markdown"))
                .arg(Arg::new("telemetry").long("telemetry").action(ArgAction::SetTrue).conflicts_with_all(["list-presets", "list-site-groups", "explain"]).help("Write local evaluation timing and aggregate counts as JSON to stderr"))
                .arg(Arg::new("fix").long("fix").action(ArgAction::SetTrue).help("Apply supported safe fixes to a local file"))
                .arg(Arg::new("no-text").long("no-text").action(ArgAction::SetTrue).help("Do not read a text input"))
                .arg(Arg::new("list-site-groups").long("list-site-groups").action(ArgAction::SetTrue).help("List native website groups usable in presets"))
                .arg(Arg::new("list-presets").long("list-presets").action(ArgAction::SetTrue).help("List available preset names"))
                .arg(Arg::new("explain").long("explain").action(ArgAction::SetTrue).help("Print the resolved preset as TOML (or JSON with --format json) without reading content"))
                .arg(Arg::new("format").long("format").value_parser(["text", "json"]).default_value("text")),
        )
        .subcommand(
            Command::new("check")
                .about("Run static checks against a site directory")
                .arg(Arg::new("path").default_value("."))
                .arg(Arg::new("config").long("config").num_args(1))
                .arg(Arg::new("baseline").long("baseline").num_args(1))
                .arg(
                    Arg::new("regressions-only")
                        .long("regressions-only")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json", "sarif"])
                        .default_value("text"),
                )
                .arg(
                    Arg::new("a11y-strict")
                        .long("a11y-strict")
                        .action(ArgAction::SetTrue)
                        .help(
                            "Strict mode for the accessibility axis: A11Y001 fires on every <img> without alt regardless of role/aria-hidden. Default is smart mode (skips canonically decorative images). Equivalent to [accessibility].strict = true in config.",
                        ),
                ),
        )
        .subcommand(
            Command::new("crawl")
                .about("Run runtime checks against a served website")
                .arg(Arg::new("url").required(true))
                .arg(
                    Arg::new("seed")
                        .long("seed")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(Arg::new("seed-file").long("seed-file").num_args(1).help("Seed every page URL on the target's origin from a link inventory JSON file and record a crawl attestation in the audit artifact"))
                .arg(
                    Arg::new("include-pattern")
                        .long("include-pattern")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("exclude-pattern")
                        .long("exclude-pattern")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("no-sitemap-seed")
                        .long("no-sitemap-seed")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("ignore-robots")
                        .long("ignore-robots")
                        .action(ArgAction::SetTrue)
                        .help("Crawl URLs that robots.txt disallows; only for auditing a site you own"),
                )
                .arg(
                    Arg::new("max-pages")
                        .long("max-pages")
                        .value_parser(value_parser!(usize))
                        .default_value("200"),
                )
                .arg(Arg::new("link-graph-output").long("link-graph-output").num_args(1).help("Write an observational link graph report to this JSON file"))
                .arg(Arg::new("link-graph-summary").long("link-graph-summary").requires("link-graph-output").action(ArgAction::SetTrue).help("Include an observational link graph coverage summary in the crawl output"))
                .arg(Arg::new("link-graph-telemetry").long("link-graph-telemetry").requires("link-graph-output").action(ArgAction::SetTrue).help("Emit aggregate link graph timing and counts to stderr"))
                .arg(Arg::new("checkpoint").long("checkpoint").num_args(1))
                .arg(Arg::new("resume").long("resume").num_args(1))
                .arg(
                    Arg::new("checkpoint-every")
                        .long("checkpoint-every")
                        .value_parser(value_parser!(usize))
                        .default_value("25"),
                )
                .arg(
                    Arg::new("artifact-every")
                        .long("artifact-every")
                        .value_parser(value_parser!(usize))
                        .default_value("25"),
                )
                .arg(
                    Arg::new("artifact-min-interval-ms")
                        .long("artifact-min-interval-ms")
                        .value_parser(value_parser!(u64))
                        .default_value("15000"),
                )
                .arg(
                    Arg::new("partial-audit-every")
                        .long("partial-audit-every")
                        .value_parser(value_parser!(usize))
                        .default_value("100"),
                )
                .arg(
                    Arg::new("partial-audit-min-interval-ms")
                        .long("partial-audit-min-interval-ms")
                        .value_parser(value_parser!(u64))
                        .default_value("60000"),
                )
                .arg(
                    Arg::new("retry-budget")
                        .long("retry-budget")
                        .value_parser(value_parser!(usize))
                        .default_value("2"),
                )
                .arg(
                    Arg::new("progress")
                        .long("progress")
                        .value_parser(["plain", "json", "off"])
                        .default_value("plain"),
                )
                .arg(Arg::new("config").long("config").num_args(1))
                .arg(
                    Arg::new("perf-budget")
                        .long("perf-budget")
                        .num_args(1)
                        .help("Fail the runtime command when a JSON performance budget is exceeded"),
                )
                .arg(Arg::new("baseline").long("baseline").num_args(1))
                .arg(
                    Arg::new("regressions-only")
                        .long("regressions-only")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("engine")
                        .long("engine")
                        .value_parser(["auto", "http", "playwright"]),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json", "sarif"])
                        .default_value("text"),
                )
                .arg(
                    Arg::new("cf-access-id")
                        .long("cf-access-id")
                        .num_args(1)
                        .help(
                            "Cloudflare Access service-token Client ID (or env CF_ACCESS_CLIENT_ID); injected as CF-Access-Client-Id header on every fetch",
                        ),
                )
                .arg(
                    Arg::new("cf-access-secret")
                        .long("cf-access-secret")
                        .num_args(1)
                        .help(
                            "Cloudflare Access service-token Client Secret (or env CF_ACCESS_CLIENT_SECRET); injected as CF-Access-Client-Secret header on every fetch",
                        ),
                )
                .arg(
                    Arg::new("a11y-strict")
                        .long("a11y-strict")
                        .action(ArgAction::SetTrue)
                        .help(
                            "Strict mode for the accessibility axis: A11Y001 fires on every <img> without alt regardless of role/aria-hidden. Default is smart mode (skips canonically decorative images). Equivalent to [accessibility].strict = true in config.",
                        ),
                ),
        )
        .subcommand(
            Command::new("config")
                .about("Inspect resolved configuration")
                .subcommand(
                    Command::new("print")
                        .about("Render the resolved config after extends and defaults")
                        .arg(Arg::new("path").default_value("."))
                        .arg(Arg::new("config").long("config").num_args(1))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["toml", "json"])
                                .default_value("toml"),
                        ),
                ),
        )
        .subcommand(
            Command::new("quality")
                .about("Run self-quality checks against an Aexeo repository")
                .arg(Arg::new("path").default_value("."))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json", "sarif"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("generate")
                .about("Generate deterministic SEO/GEO artifacts")
                .arg(Arg::new("kind").required(true).value_parser([
                    "llms",
                    "llms-full",
                    "public-bundle",
                    "markdown-mirror",
                    "markdown-pages",
                    "robots",
                    "sitemap",
                    "links",
                    "facts-prompt",
                    "schema",
                ]))
                .arg(Arg::new("path").default_value("."))
                .arg(Arg::new("config").long("config").num_args(1))
                .arg(Arg::new("write-dir").long("write-dir").num_args(1))
                .arg(Arg::new("site-url").long("site-url").num_args(1))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("snippet")
                .about("Inspect snippet-control eligibility on a built site or live URL")
                .subcommand(
                    Command::new("inspect")
                        .about("Inspect snippet controls for one route or URL")
                        .arg(Arg::new("path").long("path").num_args(1))
                        .arg(Arg::new("url").long("url").num_args(1))
                        .arg(Arg::new("route").long("route").num_args(1))
                        .arg(Arg::new("config").long("config").num_args(1))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                ),
        )
        .subcommand(
            Command::new("intelligence")
                .about("Run higher-level GEO intelligence analyses")
                .subcommand(
                    Command::new("grounding-map")
                        .about("Infer page topics, grounding intents, and answer-coverage gaps")
                        .arg(Arg::new("path").default_value("."))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                )
                .subcommand(
                    Command::new("identity")
                        .about("Compute the canonical identity Aexeo derives for a route")
                        .arg(
                            Arg::new("route")
                                .required(true)
                                .help("Route to inspect; \"\" or \"/\" for the home page"),
                        )
                        .arg(Arg::new("path").default_value("."))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                )
                .subcommand(
                    Command::new("evidence")
                        .about("Assess evidence coverage and citation fidelity risk")
                        .subcommand(
                            Command::new("assess")
                                .about("Assess claim support, evidence density, and fidelity risk")
                                .arg(Arg::new("path").default_value("."))
                                .arg(
                                    Arg::new("format")
                                        .long("format")
                                        .value_parser(["text", "json"])
                                        .default_value("text"),
                                ),
                        ),
                )
                .subcommand(
                    Command::new("fanout")
                        .about("Assess deterministic answer-engine query fan-out coverage")
                        .subcommand(
                            Command::new("assess")
                                .about("Generate and score definition, setup, comparison, pricing, trust, and support fan-out queries")
                                .arg(Arg::new("path").default_value("."))
                                .arg(
                                    Arg::new("from-crawl-artifact")
                                        .long("from-crawl-artifact")
                                        .num_args(1)
                                        .help("Read site input from a crawl audit artifact instead of scanning a directory"),
                                )
                                .arg(
                                    Arg::new("format")
                                        .long("format")
                                        .value_parser(["text", "json"])
                                        .default_value("text"),
                                ),
                        ),
                )
                .subcommand(
                    Command::new("facts")
                        .visible_alias("truth")
                        .about("Assess structured facts readiness and cross-surface consistency")
                        .subcommand(
                            Command::new("validate")
                                .about("Validate the Aexeo facts manifest contract")
                                .arg(Arg::new("path").default_value("."))
                                .arg(Arg::new("manifest").long("manifest").num_args(1))
                                .arg(
                                    Arg::new("format")
                                        .long("format")
                                        .value_parser(["text", "json"])
                                        .default_value("text"),
                                ),
                        )
                        .subcommand(
                            Command::new("generate")
                                .about("Generate a review-first facts manifest draft from site data")
                                .arg(Arg::new("path").default_value("."))
                                .arg(Arg::new("write").long("write").num_args(1))
                                .arg(
                                    Arg::new("curate")
                                        .long("curate")
                                        .action(clap::ArgAction::SetTrue),
                                )
                                .arg(
                                    Arg::new("deploy-location")
                                        .long("deploy-location")
                                        .value_parser(["root", "well-known"]),
                                )
                                .arg(
                                    Arg::new("format")
                                        .long("format")
                                        .value_parser(["text", "json"])
                                        .default_value("text"),
                                ),
                        )
                        .subcommand(
                            Command::new("assess")
                                .about("Assess schema and optional facts manifest consistency")
                                .arg(Arg::new("path").default_value("."))
                                .arg(Arg::new("manifest").long("manifest").num_args(1))
                                .arg(
                                    Arg::new("format")
                                        .long("format")
                                        .value_parser(["text", "json"])
                                        .default_value("text"),
                                ),
                        ),
                )
                .subcommand(
                    Command::new("presence")
                        .about("Surface what the open web shows about the manifest's organization (5 sources, no scoring)")
                        .arg(Arg::new("path").default_value("."))
                        .arg(Arg::new("manifest").long("manifest").num_args(1))
                        .arg(
                            Arg::new("wikipedia-language")
                                .long("wikipedia-language")
                                .num_args(1)
                                .default_value(DEFAULT_WIKIPEDIA_LANGUAGE)
                                .value_parser(parse_wikipedia_language)
                                .help("Wikipedia edition to search, such as fr or pt-br; other editions are not checked"),
                        )
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                )
                .subcommand(
                    Command::new("trust-surface")
                        .about("Import and reconcile trusted external surfaces against site truth")
                        .subcommand(
                            Command::new("import")
                                .about("Normalize trust-surface CSV or JSON records")
                                .arg(Arg::new("path").required(true))
                                .arg(Arg::new("root").long("root").num_args(1))
                                .arg(
                                    Arg::new("format")
                                        .long("format")
                                        .value_parser(["text", "json"])
                                        .default_value("text"),
                                ),
                        )
                        .subcommand(
                            Command::new("reconcile")
                                .about("Compare imported trust surfaces against site routes and truth")
                                .arg(Arg::new("input").required(true))
                                .arg(Arg::new("path").default_value("."))
                                .arg(Arg::new("manifest").long("manifest").num_args(1))
                                .arg(Arg::new("site_url").long("site-url").num_args(1))
                                .arg(
                                    Arg::new("format")
                                        .long("format")
                                        .value_parser(["text", "json"])
                                        .default_value("text"),
                                ),
                        ),
                )
                .subcommand(
                    Command::new("surfaces")
                        .about("Discover machine-readable website surfaces and retrieval paths")
                        .subcommand(
                            Command::new("discover")
                                .about("Build a machine-surface graph for HTML, schema, facts, llms, markdown, sitemap, and robots")
                                .arg(Arg::new("path").default_value("."))
                                .arg(
                                    Arg::new("from-crawl-artifact")
                                        .long("from-crawl-artifact")
                                        .num_args(1)
                                        .help("Read site input from a crawl audit artifact instead of scanning a directory"),
                                )
                                .arg(Arg::new("site-url").long("site-url").num_args(1))
                                .arg(
                                    Arg::new("format")
                                        .long("format")
                                        .value_parser(["text", "json"])
                                        .default_value("text"),
                                ),
                        ),
                )
                .subcommand(
                    Command::new("score")
                        .about("Report site and route intelligence facts and blockers (no composite score)")
                        .arg(Arg::new("path").default_value("."))
                        .arg(Arg::new("manifest").long("manifest").num_args(1))
                        .arg(Arg::new("trust-surfaces").long("trust-surfaces").num_args(1))
                        .arg(Arg::new("site-url").long("site-url").num_args(1))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                ),
        )
        .subcommand(
            Command::new("indexnow")
                .about("Validate or submit IndexNow notifications")
                .subcommand(
                    Command::new("validate")
                        .about("Validate IndexNow key format and key-file placement")
                        .arg(Arg::new("site_url").required(true))
                        .arg(Arg::new("key").required(true))
                        .arg(Arg::new("path").long("path").num_args(1))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                )
                .subcommand(
                    Command::new("plan")
                        .about("Dry-run an IndexNow payload and validate submit readiness")
                        .arg(Arg::new("endpoint").required(true))
                        .arg(Arg::new("site_url").required(true))
                        .arg(Arg::new("key").required(true))
                        .arg(Arg::new("path").long("path").num_args(1))
                        .arg(
                            Arg::new("url")
                                .required(true)
                                .num_args(1..)
                                .action(ArgAction::Append),
                        )
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                )
                .subcommand(
                    Command::new("submit")
                        .about("Submit changed URLs to an IndexNow endpoint")
                        .arg(Arg::new("endpoint").required(true))
                        .arg(Arg::new("site_url").required(true))
                        .arg(Arg::new("key").required(true))
                        .arg(Arg::new("path").long("path").num_args(1))
                        .arg(
                            Arg::new("url")
                                .required(true)
                                .num_args(1..)
                                .action(ArgAction::Append),
                        )
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                )
                .subcommand(
                    Command::new("ledger")
                        .about("Inspect the local IndexNow submission ledger")
                        .arg(Arg::new("path").default_value("."))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                )
                .subcommand(
                    Command::new("retry")
                        .about("Retry failed retryable IndexNow submissions from the ledger")
                        .arg(Arg::new("path").long("path").num_args(1).default_value("."))
                        .arg(Arg::new("key").required(true))
                        .arg(
                            Arg::new("limit")
                                .long("limit")
                                .value_parser(value_parser!(usize))
                                .default_value("10"),
                        )
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                ),
        )
        .subcommand(
            Command::new("bing-ai")
                .about("Import Bing AI Performance exports and align them with audit findings")
                .subcommand(
                    Command::new("import")
                        .about("Import a Bing AI export from CSV or JSON")
                        .arg(Arg::new("path").required(true))
                        .arg(Arg::new("audit").long("audit").num_args(1))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                )
                .subcommand(
                    Command::new("opportunities")
                        .about("Rank cited Bing AI URLs by audit severity and exposure")
                        .arg(Arg::new("path").required(true))
                        .arg(Arg::new("audit").long("audit").required(true).num_args(1))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                )
                .subcommand(
                    Command::new("trend")
                        .about("Persist and compare Bing AI import history")
                        .subcommand(
                            Command::new("import")
                                .about("Record one Bing AI export into local trend history")
                                .arg(Arg::new("path").required(true))
                                .arg(Arg::new("root").long("root").default_value("."))
                                .arg(Arg::new("audit").long("audit").num_args(1))
                                .arg(
                                    Arg::new("format")
                                        .long("format")
                                        .value_parser(["text", "json"])
                                        .default_value("text"),
                                ),
                        )
                        .subcommand(
                            Command::new("show")
                                .about("Show the latest Bing AI trend comparison")
                                .arg(Arg::new("path").default_value("."))
                                .arg(
                                    Arg::new("format")
                                        .long("format")
                                        .value_parser(["text", "json"])
                                        .default_value("text"),
                                ),
                        ),
                ),
        )
        .subcommand(
            Command::new("search-console")
                .about("Export audit findings into Search Console-oriented URL summaries")
                .subcommand(
                    Command::new("export")
                        .about("Export one audit artifact as route-level rows")
                        .arg(Arg::new("audit").required(true))
                        .arg(Arg::new("site_url").long("site-url").num_args(1))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json", "csv"])
                                .default_value("text"),
                        ),
                ),
        )
        .subcommand(
            Command::new("publish-hook")
                .about("Run post-publish checks and optional freshness notifications")
                .subcommand(
                    Command::new("run")
                        .about("Audit changed URLs, export Search Console rows, and optionally notify IndexNow")
                        .arg(Arg::new("path").default_value("."))
                        .arg(Arg::new("config").long("config").num_args(1))
                        .arg(
                            Arg::new("changed-url")
                                .long("changed-url")
                                .num_args(1)
                                .action(ArgAction::Append),
                        )
                        .arg(Arg::new("indexnow-key").long("indexnow-key").num_args(1))
                        .arg(
                            Arg::new("submit-indexnow")
                                .long("submit-indexnow")
                                .action(ArgAction::SetTrue),
                        )
                        .arg(
                            Arg::new("indexnow-endpoint")
                                .long("indexnow-endpoint")
                                .num_args(1)
                                .default_value("https://api.indexnow.org/indexnow"),
                        )
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                ),
        )
        .subcommand(
            Command::new("docs")
                .about("Generate or verify code-derived repository docs")
                .arg(
                    Arg::new("action")
                        .required(true)
                        .value_parser(["generate", "check"]),
                )
                .arg(Arg::new("path").default_value("."))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("baseline")
                .about("Save a baseline audit for later regression comparison")
                .arg(Arg::new("path").default_value("."))
                .arg(Arg::new("config").long("config").num_args(1))
                .arg(Arg::new("output").long("output").num_args(1))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("verify")
                .about("Run post-deploy verification and compare to a baseline")
                .arg(Arg::new("url").required(true))
                .arg(
                    Arg::new("seed")
                        .long("seed")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("include-pattern")
                        .long("include-pattern")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("exclude-pattern")
                        .long("exclude-pattern")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("no-sitemap-seed")
                        .long("no-sitemap-seed")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("ignore-robots")
                        .long("ignore-robots")
                        .action(ArgAction::SetTrue)
                        .help("Crawl URLs that robots.txt disallows; only for auditing a site you own"),
                )
                .arg(Arg::new("config").long("config").num_args(1))
                .arg(Arg::new("baseline").long("baseline").num_args(1))
                .arg(
                    Arg::new("allow-partial-baseline")
                        .long("allow-partial-baseline")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("max-pages")
                        .long("max-pages")
                        .value_parser(value_parser!(usize))
                        .default_value("200"),
                )
                .arg(Arg::new("checkpoint").long("checkpoint").num_args(1))
                .arg(Arg::new("resume").long("resume").num_args(1))
                .arg(
                    Arg::new("checkpoint-every")
                        .long("checkpoint-every")
                        .value_parser(value_parser!(usize))
                        .default_value("25"),
                )
                .arg(
                    Arg::new("artifact-every")
                        .long("artifact-every")
                        .value_parser(value_parser!(usize))
                        .default_value("25"),
                )
                .arg(
                    Arg::new("artifact-min-interval-ms")
                        .long("artifact-min-interval-ms")
                        .value_parser(value_parser!(u64))
                        .default_value("15000"),
                )
                .arg(
                    Arg::new("partial-audit-every")
                        .long("partial-audit-every")
                        .value_parser(value_parser!(usize))
                        .default_value("100"),
                )
                .arg(
                    Arg::new("partial-audit-min-interval-ms")
                        .long("partial-audit-min-interval-ms")
                        .value_parser(value_parser!(u64))
                        .default_value("60000"),
                )
                .arg(
                    Arg::new("retry-budget")
                        .long("retry-budget")
                        .value_parser(value_parser!(usize))
                        .default_value("2"),
                )
                .arg(
                    Arg::new("progress")
                        .long("progress")
                        .value_parser(["plain", "json", "off"])
                        .default_value("plain"),
                )
                .arg(
                    Arg::new("engine")
                        .long("engine")
                        .value_parser(["auto", "http", "playwright"]),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("profile")
                .about("Profile runtime crawl performance for a live site")
                .subcommand(
                    Command::new("runtime")
                        .about("Profile runtime crawl phases and rule-group costs")
                        .arg(Arg::new("url").required(true))
                        .arg(
                            Arg::new("seed")
                                .long("seed")
                                .num_args(1)
                                .action(ArgAction::Append),
                        )
                        .arg(
                            Arg::new("include-pattern")
                                .long("include-pattern")
                                .num_args(1)
                                .action(ArgAction::Append),
                        )
                        .arg(
                            Arg::new("exclude-pattern")
                                .long("exclude-pattern")
                                .num_args(1)
                                .action(ArgAction::Append),
                        )
                        .arg(
                            Arg::new("no-sitemap-seed")
                                .long("no-sitemap-seed")
                                .action(ArgAction::SetTrue),
                        )
                        .arg(
                            Arg::new("ignore-robots")
                                .long("ignore-robots")
                                .action(ArgAction::SetTrue)
                                .help("Crawl URLs that robots.txt disallows; only for auditing a site you own"),
                        )
                        .arg(Arg::new("config").long("config").num_args(1))
                        .arg(
                            Arg::new("perf-budget")
                                .long("perf-budget")
                                .num_args(1)
                                .help("Fail the profile command when a JSON performance budget is exceeded"),
                        )
                        .arg(
                            Arg::new("max-pages")
                                .long("max-pages")
                                .value_parser(value_parser!(usize))
                                .default_value("20"),
                        )
                        .arg(
                            Arg::new("artifact-every")
                                .long("artifact-every")
                                .value_parser(value_parser!(usize))
                                .default_value("25"),
                        )
                        .arg(
                            Arg::new("artifact-min-interval-ms")
                                .long("artifact-min-interval-ms")
                                .value_parser(value_parser!(u64))
                                .default_value("15000"),
                        )
                        .arg(
                            Arg::new("partial-audit-every")
                                .long("partial-audit-every")
                                .value_parser(value_parser!(usize))
                                .default_value("100"),
                        )
                        .arg(
                            Arg::new("partial-audit-min-interval-ms")
                                .long("partial-audit-min-interval-ms")
                                .value_parser(value_parser!(u64))
                                .default_value("60000"),
                        )
                        .arg(
                            Arg::new("retry-budget")
                                .long("retry-budget")
                                .value_parser(value_parser!(usize))
                                .default_value("2"),
                        )
                        .arg(Arg::new("engine").long("engine").value_parser([
                            "auto",
                            "http",
                            "playwright",
                        ]))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                ),
        )
        .subcommand(
            Command::new("perf")
                .about("Compare and inspect runtime performance artifacts")
                .subcommand(
                    Command::new("baseline")
                        .about("Run a runtime crawl and save a named performance baseline")
                        .arg(Arg::new("url").required(true))
                        .arg(
                            Arg::new("name")
                                .long("name")
                                .num_args(1)
                                .default_value("runtime")
                                .help("Stable baseline name used for .aexeo-reports artifacts"),
                        )
                        .arg(
                            Arg::new("seed")
                                .long("seed")
                                .num_args(1)
                                .action(ArgAction::Append),
                        )
                        .arg(
                            Arg::new("include-pattern")
                                .long("include-pattern")
                                .num_args(1)
                                .action(ArgAction::Append),
                        )
                        .arg(
                            Arg::new("exclude-pattern")
                                .long("exclude-pattern")
                                .num_args(1)
                                .action(ArgAction::Append),
                        )
                        .arg(
                            Arg::new("no-sitemap-seed")
                                .long("no-sitemap-seed")
                                .action(ArgAction::SetTrue),
                        )
                        .arg(
                            Arg::new("ignore-robots")
                                .long("ignore-robots")
                                .action(ArgAction::SetTrue)
                                .help("Crawl URLs that robots.txt disallows; only for auditing a site you own"),
                        )
                        .arg(Arg::new("config").long("config").num_args(1))
                        .arg(
                            Arg::new("perf-budget")
                                .long("perf-budget")
                                .num_args(1)
                                .help("JSON performance budget; defaults to configured performance_budget_file when present"),
                        )
                        .arg(
                            Arg::new("compare-to")
                                .long("compare-to")
                                .num_args(1)
                                .help("Baseline artifact to compare before updating the named latest baseline"),
                        )
                        .arg(
                            Arg::new("promote-on-regression")
                                .long("promote-on-regression")
                                .action(ArgAction::SetTrue)
                                .help("Update the named latest baseline even when budget or diff gates fail"),
                        )
                        .arg(
                            Arg::new("max-pages")
                                .long("max-pages")
                                .value_parser(value_parser!(usize))
                                .default_value("200"),
                        )
                        .arg(
                            Arg::new("retry-budget")
                                .long("retry-budget")
                                .value_parser(value_parser!(usize))
                                .default_value("2"),
                        )
                        .arg(Arg::new("engine").long("engine").value_parser([
                            "auto",
                            "http",
                            "playwright",
                        ]))
                        .arg(
                            Arg::new("regression-threshold-pct")
                                .long("regression-threshold-pct")
                                .value_parser(value_parser!(u32))
                                .default_value("10")
                                .help("Minimum relative delta percentage before a metric is marked regressed"),
                        )
                        .arg(
                            Arg::new("absolute-threshold-ms")
                                .long("absolute-threshold-ms")
                                .value_parser(value_parser!(u64))
                                .default_value("25")
                                .help("Minimum absolute millisecond delta before timing metrics are marked regressed"),
                        )
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                )
                .subcommand(
                    Command::new("diff")
                        .about("Compare runtime performance between two audit artifacts")
                        .arg(Arg::new("baseline").required(true))
                        .arg(Arg::new("current").required(true))
                        .arg(
                            Arg::new("regression-threshold-pct")
                                .long("regression-threshold-pct")
                                .value_parser(value_parser!(u32))
                                .default_value("10")
                                .help("Minimum relative delta percentage before a metric is marked regressed"),
                        )
                        .arg(
                            Arg::new("absolute-threshold-ms")
                                .long("absolute-threshold-ms")
                                .value_parser(value_parser!(u64))
                                .default_value("0")
                                .help("Minimum absolute millisecond delta before timing metrics are marked regressed"),
                        )
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                ),
        )
        .subcommand(
            Command::new("report")
                .about("Render an audit artifact into a human or machine readable report")
                .subcommand(
                    Command::new("render")
                        .about("Render an audit JSON artifact")
                        .arg(Arg::new("audit").required(true))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "md", "json", "sarif"])
                                .default_value("text"),
                        ),
                ),
        )
        .subcommand(
            Command::new("doctor")
                .about("Inspect local runtime capabilities")
                .arg(Arg::new("target").required(true).value_parser(["runtime"]))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("diff")
                .about("Compare two audit artifacts and report regressions")
                .arg(Arg::new("baseline").required(true))
                .arg(Arg::new("current").required(true))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("facts")
                .about("Author and validate truth manifests (facts.json)")
                .subcommand(
                    Command::new("validate")
                        .about("Validate a candidate facts.json against shape and site")
                        .arg(Arg::new("path").required(true))
                        .arg(
                            Arg::new("site-path")
                                .long("site-path")
                                .num_args(1)
                                .help("Path to the static site root used for mismatch detection. Defaults to current directory."),
                        )
                        .arg(Arg::new("config").long("config").num_args(1))
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_parser(["text", "json"])
                                .default_value("text"),
                        ),
                ),
        )
        .subcommand(
            Command::new("trend")
                .about("Show recent audit trend history for a command")
                .arg(
                    Arg::new("command_name")
                        .required(true)
                        .value_parser(["check", "crawl", "quality"]),
                )
                .arg(Arg::new("path").default_value("."))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("fix")
                .about("Apply safe deterministic fixes")
                .arg(Arg::new("path").default_value("."))
                .arg(Arg::new("config").long("config").num_args(1))
                .arg(
                    Arg::new("inject-schema")
                        .long("inject-schema")
                        .action(clap::ArgAction::SetTrue)
                        .help(
                            "Read schema-suggestions.json from the path and inject suggested JSON-LD into HTML <head> blocks (per-type augmentation; existing types are skipped).",
                        ),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .action(clap::ArgAction::SetTrue)
                        .help("List the files fix would change or skip without writing anything."),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("rules")
                .about("List built-in rule groups")
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("adapters")
                .about("List registered site adapters")
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("plugin-check")
                .about("Validate one plugin module manifest and compatibility")
                .arg(Arg::new("module_name").required(true))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
}

pub fn render_cli_reference() -> Result<String> {
    let cli = build_cli();
    let mut lines = vec![
        "# CLI Reference".to_string(),
        String::new(),
        "<!-- Generated by aexeo-cli docs generate. Do not edit by hand. -->".to_string(),
        String::new(),
        "## Commands".to_string(),
        String::new(),
    ];
    for command in cli.get_subcommands() {
        let mut buffer = Vec::new();
        let mut subcommand = command.clone();
        subcommand.write_long_help(&mut buffer)?;
        lines.push(format!("## `{}`", command.get_name()));
        lines.push(String::new());
        if let Some(about) = command.get_about() {
            lines.push(about.to_string());
            lines.push(String::new());
        }
        lines.push("```text".to_string());
        lines.push(
            String::from_utf8(buffer)
                .context("CLI help should be UTF-8")?
                .lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string(),
        );
        lines.push("```".to_string());
        lines.push(String::new());
    }
    Ok(lines.join("\n"))
}

fn parse_wikipedia_language(value: &str) -> Result<String, String> {
    if is_wikipedia_language(value) {
        Ok(value.to_string())
    } else {
        Err("expected a Wikipedia edition code such as en, fr or pt-br".to_string())
    }
}
