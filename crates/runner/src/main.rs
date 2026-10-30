// SPDX-License-Identifier: Apache-2.0
//! The `ninge-runner` binary: parse arguments, load the ROM, run, print one report line.

use std::process::ExitCode;

use ninge_runner::{RunnerError, load_rom_file, parse_args, run};

fn main() -> ExitCode {
    match execute() {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("ninge-runner: {error}");
            ExitCode::from(exit_code(&error))
        }
    }
}

fn execute() -> Result<String, RunnerError> {
    let args = parse_args(std::env::args_os().skip(1))?;
    let rom = load_rom_file(args.rom())?;
    Ok(run(rom, args.frames()).to_string())
}

/// Usage errors exit 2, IO errors exit 1.
fn exit_code(error: &RunnerError) -> u8 {
    match error {
        RunnerError::Usage(_) => 2,
        RunnerError::Io { .. } => 1,
    }
}
