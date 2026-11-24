// SPDX-License-Identifier: Apache-2.0
//! The `ninge-runner` binary: parse arguments, load the ROM, run, print one report line.

use std::process::ExitCode;

use ninge_core::cart::SaveType;
use ninge_runner::cpu_suite::report_dir;
use ninge_runner::replay::replay;
use ninge_runner::{
    Command, RunnerError, default_save_path, load_rom_file, parse_command, read_save_file,
    run_with_audio, run_with_save, write_wav,
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
        Command::Replay(args) => {
            let rom = load_rom_file(args.rom())?;
            let log = std::fs::read_to_string(args.log()).map_err(|source| RunnerError::Io {
                path: args.log().to_path_buf(),
                source,
            })?;
            let save_path = args
                .save()
                .map_or_else(|| default_save_path(args.rom()), ToOwned::to_owned);
            let outcome = replay(rom, &log, args.frames(), read_save_file(&save_path)?)?;
            if outcome.dropped_partial_line {
                // On stderr, so stdout stays the one report line.
                eprintln!("ninge-runner: the log ends in a partial line; it was dropped");
            }
            Ok(outcome.report.to_string())
        }
        Command::CpuSuite(dir) => Ok(report_dir(&dir)?),
    }
}

/// Usage errors exit 2, a refused ROM exits 3, a refused save exits 4, a refused input log exits
/// 5, IO errors exit 1.
fn exit_code(error: &RunnerError) -> u8 {
    match error {
        RunnerError::Usage(_) => 2,
        RunnerError::Rom(_) => 3,
        RunnerError::Save(_) => 4,
        RunnerError::Log(_) | RunnerError::LogMismatch(_) => 5,
        RunnerError::Io { .. } | RunnerError::Write { .. } | RunnerError::Suite(_) => 1,
    }
}
