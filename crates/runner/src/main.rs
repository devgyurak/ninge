// SPDX-License-Identifier: Apache-2.0
//! The `ninge-runner` binary: parse arguments, load the ROM, run, print one report line.

use std::process::ExitCode;

use ninge_runner::cpu_suite::report_dir;
use ninge_runner::{Command, RunnerError, load_rom_file, parse_command, run};

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
    match parse_command(std::env::args_os().skip(1))? {
        Command::Run(args) => {
            let rom = load_rom_file(args.rom())?;
            Ok(run(rom, args.frames()).to_string())
        }
        Command::CpuSuite(dir) => Ok(report_dir(&dir)?),
    }
}

/// Usage errors exit 2, IO errors exit 1.
fn exit_code(error: &RunnerError) -> u8 {
    match error {
        RunnerError::Usage(_) => 2,
        RunnerError::Io { .. } | RunnerError::Suite(_) => 1,
    }
}
