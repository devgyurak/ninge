// SPDX-License-Identifier: Apache-2.0
//! The `ninge` binary.

use ninge::source::{StartError, open_recording, start as start_source};
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
    let options = ninge::cli::parse_args(std::env::args_os().skip(1)).map_err(StartError::Usage)?;
    let (source, save) = start_source(&options)?;
    let recorder = open_recording(&options, &source)?;
    let note = ninge::term::run(
        source,
        save,
        recorder,
        !options.no_audio,
        options.focus_pause,
    )
    .map_err(|e| StartError::Run(format!("{e:#}")))?;
    // After the terminal is restored, so a wrong detection is visible to the user.
    if let Some(note) = note {
        eprintln!("ninge: {note}");
    }
    Ok(())
}
