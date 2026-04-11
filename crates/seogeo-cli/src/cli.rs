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
                    Arg::new("max-pages")
                        .long("max-pages")
                        .value_parser(value_parser!(usize))
                        .default_value("200"),
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
                .trim()
                .to_string(),
        );
        lines.push("```".to_string());
        lines.push(String::new());
    }
    Ok(lines.join("\n"))
}
