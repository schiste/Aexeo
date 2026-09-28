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

/// Command succeeded and there is nothing to act on.
pub const EXIT_SUCCESS: i32 = 0;

/// Command succeeded, but the run produced blocking findings or regressions.
///
/// This is a *result*, not a failure: the command did its job and the answer
/// is "there is work to do".
pub const EXIT_FINDINGS: i32 = 1;

/// Command could not be carried out as asked: missing configuration, an
/// unsupported input state, an empty artifact that was refused on purpose.
///
/// Also a result. Retrying unchanged will not help.
pub const EXIT_UNSUPPORTED: i32 = 2;

/// Aexeo itself failed: an unexpected error propagated out of a command.
///
/// Distinct from every result code so automation can separate "your site has
/// a problem" from "the tool has a problem". 70 is `EX_SOFTWARE` from
/// `sysexits.h`, the conventional code for an internal software error.
pub const EXIT_INTERNAL_ERROR: i32 = 70;

#[cfg(test)]
mod tests {
    use super::{EXIT_FINDINGS, EXIT_INTERNAL_ERROR, EXIT_SUCCESS, EXIT_UNSUPPORTED};

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

    /// The regression this module exists for.
    #[test]
    fn internal_errors_do_not_collide_with_findings() {
        assert_ne!(EXIT_INTERNAL_ERROR, EXIT_FINDINGS);
        assert_ne!(EXIT_INTERNAL_ERROR, EXIT_UNSUPPORTED);
    }
}
