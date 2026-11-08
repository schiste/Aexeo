#![forbid(unsafe_code)]

mod cli;
mod commands;
mod output;

use std::process::ExitCode;

use commands::common::report_failure;
use commands::exit_code::exit_code_for_error;
use commands::utility::{dispatch, invocation};

fn main() -> ExitCode {
    let matches = cli::build_cli().get_matches();
    let (command, format) = invocation(&matches);
    let code = dispatch(matches).unwrap_or_else(|error| {
        if let Err(render_error) = report_failure(&command, &format, &error, Vec::new()) {
            eprintln!("{error:#}\n{render_error:#}");
        }
        // Never 1: a command that legitimately reports blocking findings
        // returns 1, and a CI gate must be able to tell "your site has
        // problems" from "aexeo failed to run". See commands/exit_code.
        exit_code_for_error(&error)
    });
    ExitCode::from(code as u8)
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
