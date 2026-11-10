// SPDX-License-Identifier: Apache-2.0
//! Command-line arguments: an optional ROM path. Pure, so it is tested without a process.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

/// The usage line shown with every [`UsageError`].
pub const USAGE: &str = "usage: ninge [<rom>]";

/// Arguments the frontend does not accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(String);

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n{USAGE}", self.0)
    }
}

impl std::error::Error for UsageError {}

/// Parses the arguments after the program name: none shows the dummy pattern, one is the
/// ROM path. More than one, or anything starting with `-`, is a usage error.
pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<PathBuf>, UsageError> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(None);
    };
    if first.to_string_lossy().starts_with('-') {
        return Err(UsageError(format!(
            "unknown option {}",
            first.to_string_lossy()
        )));
    }
    if args.next().is_some() {
        return Err(UsageError("expected at most one ROM path".to_string()));
    }
    Ok(Some(PathBuf::from(first)))
}

#[cfg(test)]
mod tests;
