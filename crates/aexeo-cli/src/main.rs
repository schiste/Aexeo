#![forbid(unsafe_code)]

mod cli;
mod commands;
mod output;

use anyhow::Result;
use std::process::ExitCode;

use commands::exit_code::EXIT_INTERNAL_ERROR;

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("{}", error);
            // Not 1: a command that legitimately reports blocking findings
            // also returns 1, and a CI gate must be able to tell "your site
            // has problems" from "aexeo failed to run". See commands/exit_code.
            ExitCode::from(EXIT_INTERNAL_ERROR as u8)
        }
    }
}

fn run() -> Result<i32> {
    commands::utility::dispatch(cli::build_cli().get_matches())
}

#[cfg(test)]
mod tests {
    use crate::cli::render_cli_reference;

    #[test]
    fn cli_reference_mentions_core_commands() {
        let reference = render_cli_reference().unwrap();
        assert!(reference.contains("## `docs`"));
        assert!(reference.contains("## `rules`"));
        assert!(reference.contains("## `quality`"));
        assert!(reference.contains("## `check`"));
    }
}
