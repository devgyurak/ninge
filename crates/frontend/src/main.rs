// SPDX-License-Identifier: Apache-2.0
//! The `ninge` binary.

use ninge::source::{Source, StartError, load};
use std::process::ExitCode;

fn main() -> ExitCode {
    match start() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ninge: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}

/// Reads and checks the ROM before the terminal changes mode, so a refusal prints on the
/// normal screen.
fn start() -> Result<(), StartError> {
    let source =
        match ninge::cli::parse_args(std::env::args_os().skip(1)).map_err(StartError::Usage)? {
            Some(path) => load(path)?,
            None => Source::Pattern,
        };
    ninge::term::run(source).map_err(|e| StartError::Run(format!("{e:#}")))
}
