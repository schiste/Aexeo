use std::{fs::File, io::Read};

use aexeo_contracts::content_gate_change_validation::parse_unique_json_value;
use aexeo_core::{
    editorial_pacing::{CompositionProfile, builtin_profile},
    error::UserContext,
};
use anyhow::{Result, anyhow};
use clap::ArgMatches;
use serde_json::json;

use crate::commands::exit_code::{EXIT_FINDINGS, EXIT_SUCCESS, EXIT_UNSUPPORTED};

const MAX_BYTES: usize = 1_100_000;

fn read_bounded(path: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(anyhow!("input exceeds resource limit"))
            .user_context("invalid composition input");
    }
    Ok(bytes)
}

pub fn command_composition(args: &ArgMatches) -> Result<i32> {
    let path = args.get_one::<String>("profile").expect("required profile");
    let profile_bytes = if let Some(profile) = builtin_profile(path) {
        profile.as_bytes().to_vec()
    } else {
        read_bounded(path)?
    };
    let text = std::str::from_utf8(&profile_bytes).user_context("profile is not UTF-8")?;
    let profile =
        CompositionProfile::from_toml(text).user_context("invalid composition profile")?;
    let telemetry = args.get_flag("telemetry");
    if args.get_flag("schema") {
        println!("{}", serde_json::to_string_pretty(&profile.json_schema()?)?);
        if telemetry {
            eprintln!(
                "{}",
                json!({"event":"composition", "version":1, "operation":"schema", "field_count":profile.fields.len()})
            );
        }
        return Ok(EXIT_SUCCESS);
    }
    if args.get_flag("emit-preset") {
        let preset = aexeo_core::content_gate::ContentPreset {
            version: 1,
            name: profile.name.clone(),
            gates: vec![aexeo_core::content_gate::ContentGate::EditorialPacing {
                id: "editorial.pacing".into(),
                severity: aexeo_core::content_gate::Severity::Warning,
                blocking: false,
                source: "composition".into(),
                profile: aexeo_core::editorial_pacing::CompositionProfileRef::Inline(profile),
            }],
        };
        print!("{}", preset.to_toml()?);
        if telemetry {
            eprintln!(
                "{}",
                json!({"event":"composition", "version":1, "operation":"emit_preset", "gate_count":1})
            );
        }
        return Ok(EXIT_SUCCESS);
    }
    let Some(input_path) = args.get_one::<String>("input") else {
        println!(
            "{}",
            json!({"status":"not_checked", "reason":"structured composition not supplied"})
        );
        if telemetry {
            eprintln!(
                "{}",
                json!({"event":"composition", "version":1, "operation":"assess", "status":"not_checked", "issue_count":0})
            );
        }
        return Ok(EXIT_UNSUPPORTED);
    };
    let input = parse_unique_json_value(&read_bounded(input_path)?)
        .map_err(|_| anyhow!("invalid or duplicate JSON field"))
        .user_context("invalid composition input")?;
    let issues = profile
        .assess(&input)
        .user_context("invalid composition input")?;
    if args.get_flag("assemble") {
        let allow_advisory = args.get_flag("allow-advisory-violations");
        let assembled = if allow_advisory {
            profile.assemble_snapshot(&input)
        } else {
            profile.assemble(&input)
        };
        let accepted = assembled.is_ok();
        if let Ok(text) = assembled {
            print!("{text}");
        } else {
            eprintln!("{}", json!({"status":"failed", "issues":issues}));
        }
        if telemetry {
            eprintln!(
                "{}",
                json!({"event":"composition", "version":1, "operation":"assemble", "issue_count":issues.len(), "advisory_override":allow_advisory})
            );
        }
        return Ok(if accepted {
            EXIT_SUCCESS
        } else {
            EXIT_FINDINGS
        });
    }
    let document = args.get_one::<String>("document");
    let matches = match (document, profile.assemble_snapshot(&input)) {
        (Some(path), Ok(text)) => Some(read_bounded(path)? == text.as_bytes()),
        _ => None,
    };
    let status = if document.is_some() && matches != Some(true) {
        "not_checked"
    } else if issues.is_empty() {
        "passed"
    } else {
        "failed"
    };
    println!(
        "{}",
        json!({"status":status, "scope":if document.is_some() {"document"} else {"structured_input"}, "issues":issues, "document_matches":matches})
    );
    if telemetry {
        eprintln!(
            "{}",
            json!({"event":"composition", "version":1, "operation":"assess", "status":status, "issue_count":issues.len(), "document_compared":matches.is_some()})
        );
    }
    Ok(match status {
        "failed" => EXIT_FINDINGS,
        "not_checked" => EXIT_UNSUPPORTED,
        _ => EXIT_SUCCESS,
    })
}
