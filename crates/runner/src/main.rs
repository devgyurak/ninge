// SPDX-License-Identifier: Apache-2.0
//! The `ninge-runner` binary: parse arguments, load the ROM, run, print one report line.

use std::process::ExitCode;

use ninge_core::cart::SaveType;
use ninge_runner::cpu_suite::report_dir;
use ninge_runner::{
    Command, RunnerError, load_rom_file, parse_command, run_with_audio, run_with_save, write_wav,
};

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
            if args.audio() {
                let (report, samples) = run_with_audio(rom, args.frames())?;
                if let Some(path) = args.wav() {
                    write_wav(path, &samples)?;
                }
                return Ok(report.to_string());
            }
            let (report, save) = run_with_save(rom, args.frames(), &args.save_setup())?;
            // On stderr, so stdout stays the one report line.
            if save.kind != SaveType::None {
                eprintln!("ninge-runner: {save}");
            }
            Ok(report.to_string())
        }
        Command::CpuSuite(dir) => Ok(report_dir(&dir)?),
        Command::Trace(args) => Ok(ninge_runner::trace::write_trace_file(&args)?.to_string()),
    }
}

/// Usage errors exit 2, a refused ROM exits 3, a refused save exits 4, IO errors exit 1.
fn exit_code(error: &RunnerError) -> u8 {
    match error {
        RunnerError::Usage(_) => 2,
        RunnerError::Rom(_) => 3,
        RunnerError::Save(_) => 4,
        RunnerError::Io { .. } | RunnerError::Write { .. } | RunnerError::Suite(_) => 1,
    }
}
