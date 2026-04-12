use anyhow::{Context, Result};
use clap::{Arg, ArgAction, Command, value_parser};

pub fn build_cli() -> Command {
    Command::new("seogeo")
        .about("SEO and GEO linting for static websites")
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
                .arg(Arg::new("config").long("config").num_args(1))
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
                .about("Run self-quality checks against a seogeo repository")
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
                    "markdown-mirror",
                    "robots",
                    "links",
                ]))
                .arg(Arg::new("path").default_value("."))
                .arg(Arg::new("config").long("config").num_args(1))
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
                    Command::new("truth")
                        .about("Assess structured truth readiness and cross-surface consistency")
                        .subcommand(
                            Command::new("assess")
                                .about("Assess schema and optional truth manifest consistency")
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
                        .arg(Arg::new("config").long("config").num_args(1))
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
        "<!-- Generated by seogeo docs generate. Do not edit by hand. -->".to_string(),
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
