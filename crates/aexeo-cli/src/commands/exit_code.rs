//! Process exit codes.
//!
//! These are part of the CLI contract (see `SPEC.md`), so they live in one
//! place rather than as bare `Ok(1)` / `Ok(2)` literals scattered through the
//! command modules.
//!
//! The important property is that every value has exactly one meaning. In
//! particular `EXIT_FINDINGS` and `EXIT_INTERNAL_ERROR` used to collide: a
//! command that found blocking findings returned `1`, and so did the top
//! level when any command returned `Err`. A CI gate could not tell "your
//! site regressed" from "aexeo crashed", and could not distinguish a real
//! result from a bug worth filing.

use aexeo_core::error::is_user_error;

/// Command succeeded and there is nothing to act on.
pub const EXIT_SUCCESS: i32 = 0;

/// Command succeeded, but the run produced blocking findings or regressions.
///
/// This is a *result*, not a failure: the command did its job and the answer
/// is "there is work to do".
pub const EXIT_FINDINGS: i32 = 1;

/// Command could not be carried out as asked: invalid usage, a bad path, an
/// unreadable or invalid input (configuration, preset, artifact, manifest), an
/// unreachable target, an unsupported input state, an empty artifact that was
/// refused on purpose.
///
/// Fix the input or the environment; retrying unchanged will rarely help.
pub const EXIT_UNSUPPORTED: i32 = 2;

/// Aexeo itself failed: an error nothing classified as the user's to fix.
///
/// Distinct from every result code so automation can separate "your site has
/// a problem" from "the tool has a problem". 70 is `EX_SOFTWARE` from
/// `sysexits.h`, the conventional code for an internal software error.
pub const EXIT_INTERNAL_ERROR: i32 = 70;

/// The exit code for an error that propagated out of a command.
///
/// An I/O failure anywhere in the cause chain counts as the environment's, not
/// a bug: Aexeo asked the operating system for something it refused or could
/// not do, whether or not the failing call site marked it as a `UserError`.
pub fn exit_code_for_error(error: &anyhow::Error) -> i32 {
    let io_failure = error
        .chain()
        .any(|cause| cause.downcast_ref::<std::io::Error>().is_some());
    if is_user_error(error) || io_failure {
        EXIT_UNSUPPORTED
    } else {
        EXIT_INTERNAL_ERROR
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EXIT_FINDINGS, EXIT_INTERNAL_ERROR, EXIT_SUCCESS, EXIT_UNSUPPORTED, exit_code_for_error,
    };
    use aexeo_core::error::UserContext;
    use anyhow::{Context, anyhow};

    #[test]
    fn exit_codes_are_distinct_and_within_the_u8_range() {
        let codes = [
            EXIT_SUCCESS,
            EXIT_FINDINGS,
            EXIT_UNSUPPORTED,
            EXIT_INTERNAL_ERROR,
        ];
        for (index, code) in codes.iter().enumerate() {
            assert!((0..=255).contains(code), "{code} is not a valid exit code");
            assert!(
                !codes[..index].contains(code),
                "{code} is used for more than one meaning"
            );
        }
    }

    #[test]
    fn only_unclassified_errors_are_internal() {
        assert_eq!(exit_code_for_error(&anyhow!("bug")), EXIT_INTERNAL_ERROR);

        let user = Err::<(), _>(anyhow!("cause"))
            .user_context("bad input")
            .context("outer")
            .unwrap_err();
        assert_eq!(exit_code_for_error(&user), EXIT_UNSUPPORTED);

        let io = Err::<(), _>(std::io::Error::from(std::io::ErrorKind::NotFound))
            .context("outer")
            .unwrap_err();
        assert_eq!(exit_code_for_error(&io), EXIT_UNSUPPORTED);
    }

    /// The regression this module exists for.
    #[test]
    fn internal_errors_do_not_collide_with_findings() {
        assert_ne!(EXIT_INTERNAL_ERROR, EXIT_FINDINGS);
        assert_ne!(EXIT_INTERNAL_ERROR, EXIT_UNSUPPORTED);
    }
}
