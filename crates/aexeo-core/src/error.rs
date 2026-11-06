//! Errors the user can act on, as opposed to defects in Aexeo.
//!
//! Commands propagate `anyhow::Error`, which on its own cannot tell "that path
//! does not exist" from "Aexeo has a bug". The CLI exit-code contract in
//! `SPEC.md` needs that distinction, so code that rejects an input marks the
//! error with [`UserError`] and the CLI maps the mark to its exit code.

use anyhow::{Error, Result};
use std::fmt::{self, Display};

/// The request could not be carried out because of its input: a bad path, an
/// unreadable or invalid file, invalid configuration or an invalid preset.
#[derive(Debug)]
pub struct UserError {
    message: String,
}

impl UserError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for UserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for UserError {}

/// Whether any layer of `error` was marked with [`UserError`].
pub fn is_user_error(error: &Error) -> bool {
    error.downcast_ref::<UserError>().is_some()
}

/// Mark `error` as a [`UserError`] whose message is its whole cause chain, for
/// failures that already describe themselves. Library callers that only print
/// the outermost message, such as the WASM bridge, still see the cause.
pub fn into_user_error(error: Error) -> Error {
    UserError::new(format!("{error:#}")).into()
}

/// Attach a message to a failure and mark it as a [`UserError`], keeping the
/// original error as its cause.
pub trait UserContext<T> {
    fn user_context(self, message: impl Display) -> Result<T>;
}

impl<T, E: Into<Error>> UserContext<T> for std::result::Result<T, E> {
    fn user_context(self, message: impl Display) -> Result<T> {
        self.map_err(|error| error.into().context(UserError::new(message.to_string())))
    }
}

impl<T> UserContext<T> for Option<T> {
    fn user_context(self, message: impl Display) -> Result<T> {
        self.ok_or_else(|| UserError::new(message.to_string()).into())
    }
}

/// `anyhow::bail!`, marked as a [`UserError`].
#[macro_export]
macro_rules! user_bail {
    ($($arg:tt)*) => {
        return ::core::result::Result::Err(
            $crate::error::UserError::new(::std::format!($($arg)*)).into(),
        )
    };
}

#[cfg(test)]
mod tests {
    use super::{UserContext, UserError, into_user_error, is_user_error};
    use anyhow::{Context, Result, anyhow};

    #[test]
    fn a_plain_error_is_not_a_user_error() {
        assert!(!is_user_error(&anyhow!("bug")));
        assert!(!is_user_error(
            &Err::<(), _>(anyhow!("bug")).context("outer").unwrap_err()
        ));
    }

    /// The mark must survive whatever a caller wraps around it, and whatever
    /// the mark itself wraps, or a nested failure silently becomes "a bug".
    #[test]
    fn the_mark_is_found_through_context_layers() {
        let direct: anyhow::Error = UserError::new("bad").into();
        assert!(is_user_error(&direct));

        let wrapped = Err::<(), _>(direct).context("outer").context("outermost");
        assert!(is_user_error(&wrapped.unwrap_err()));

        let marked = Err::<(), _>(anyhow!("cause"))
            .user_context("bad input")
            .context("outer")
            .unwrap_err();
        assert!(is_user_error(&marked));
        assert_eq!(format!("{marked:#}"), "outer: bad input: cause");
    }

    #[test]
    fn into_user_error_keeps_the_cause_in_the_outermost_message() {
        let error = into_user_error(
            Err::<(), _>(anyhow!("field error"))
                .context("bad config")
                .unwrap_err(),
        );
        assert!(is_user_error(&error));
        assert_eq!(error.to_string(), "bad config: field error");
        assert_eq!(format!("{error:#}"), "bad config: field error");
    }

    #[test]
    fn std_errors_and_options_can_be_marked() {
        let io: Result<()> =
            Err::<(), _>(std::io::Error::other("disk")).user_context("cannot read x");
        assert_eq!(format!("{:#}", io.unwrap_err()), "cannot read x: disk");

        let none = None::<()>.user_context("missing x").unwrap_err();
        assert!(is_user_error(&none));
    }

    #[test]
    fn user_bail_returns_a_marked_formatted_error() {
        fn fails(name: &str) -> Result<()> {
            user_bail!("bad {name}");
        }
        let error = fails("thing").unwrap_err();
        assert!(is_user_error(&error));
        assert_eq!(error.to_string(), "bad thing");
    }
}
