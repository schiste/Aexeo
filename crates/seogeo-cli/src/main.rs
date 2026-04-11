#![forbid(unsafe_code)]

mod cli;
mod commands;
mod output;

use anyhow::Result;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("{}", error);
            ExitCode::from(1)
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
