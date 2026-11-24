// SPDX-License-Identifier: Apache-2.0
//! Replaying an input log (`docs/contributing/input-log.md`) on our own `keypad` test ROM, whose
//! ARM9 records what KEYINPUT reads at 0x0200_4000 slot 1 (`testroms/keypad/`, `docs/hw/keypad.md`
//! "Where the bits come from").

use super::*;
use crate::{Command, load_rom_file, parse_command, testrom_path};
use ninge_core::cart::SaveType;
use ninge_core::input_log::{LogErrorKind, LogHeader, MismatchError, encode_frame, encode_header};
use ninge_core::{Buttons, Input};

const ARM9_KEYINPUT_SLOT: u32 = 0x0200_4000 + 4;

fn keypad_rom() -> Vec<u8> {
    load_rom_file(&testrom_path("keypad")).expect("keypad ROM is built")
}

fn rom_crc(rom: &[u8]) -> u16 {
    let mut machine = Machine::new();
    machine.load_rom(rom.to_vec()).expect("a valid header");
    machine.header().expect("a header is parsed").crc
}

/// A header for `rom` with no save chip.
fn header_for(rom: &[u8]) -> LogHeader {
    LogHeader {
        rom_crc: rom_crc(rom),
        save_type: SaveType::None,
        save_hash: input_log::save_hash(&[]),
    }
}

/// The text of a log of `inputs` for `rom` with no save chip.
fn log_text(rom: &[u8], inputs: &[Input]) -> String {
    inputs
        .iter()
        .enumerate()
        .fold(encode_header(&header_for(rom)), |text, (index, input)| {
            text + &encode_frame(index as u32 + 1, input)
        })
}

fn a_and_down() -> Input {
    Input::new().with_buttons(Buttons::A | Buttons::DOWN)
}

fn begin(inputs: &[Input]) -> Replay {
    let rom = keypad_rom();
    Replay::begin(rom.clone(), &log_text(&rom, inputs), None).expect("the log fits")
}

/// The mismatch a replay was refused for, or `None` for any other outcome.
fn mismatch_of(result: Result<ReplayOutcome, RunnerError>) -> Option<MismatchError> {
    result.err().and_then(|error| match error {
        RunnerError::LogMismatch(mismatch) => Some(mismatch),
        _ => None,
    })
}

/// The line and kind of the log error a replay was refused for, or `None` for any other outcome.
fn log_error_of(result: Result<ReplayOutcome, RunnerError>) -> Option<(usize, LogErrorKind)> {
    result.err().and_then(|error| match error {
        RunnerError::Log(error) => Some((error.line, error.kind)),
        _ => None,
    })
}

fn args(list: &[&str]) -> Result<ReplayArgs, RunnerError> {
    parse_replay_args(list.iter().map(OsString::from))
}

/// The usage-error cases that are not refused as a usage error.
fn usage_failures(cases: &[&[&str]]) -> Vec<Vec<String>> {
    cases
        .iter()
        .filter(|case| !matches!(args(case), Err(RunnerError::Usage(_))))
        .map(|case| case.iter().map(ToString::to_string).collect())
        .collect()
}

// ---- Running a log ----

#[test]
fn given_a_log_holding_a_and_down_when_replayed_then_the_arm9_reads_them_in_keyinput() {
    let mut replay = begin(&[a_and_down(), a_and_down()]);
    replay.run(None);
    assert_eq!(
        replay.machine().inspect().arm9_peek32(ARM9_KEYINPUT_SLOT),
        0x037E
    );
}

#[test]
fn given_a_log_with_nothing_held_when_replayed_then_the_arm9_reads_all_keys_released() {
    let mut replay = begin(&[Input::new(), Input::new()]);
    replay.run(None);
    assert_eq!(
        replay.machine().inspect().arm9_peek32(ARM9_KEYINPUT_SLOT),
        0x03FF
    );
}

#[test]
fn given_a_log_of_two_frames_when_replayed_with_no_count_then_two_frames_run() {
    assert_eq!(
        begin(&[a_and_down(), Input::new()]).run(None).report.frames,
        2
    );
}

#[test]
fn given_a_log_of_two_frames_when_replayed_for_five_then_five_frames_run() {
    assert_eq!(
        begin(&[a_and_down(), Input::new()])
            .run(Some(5))
            .report
            .frames,
        5
    );
}

#[test]
fn given_a_log_of_two_frames_when_replayed_for_one_then_one_frame_runs() {
    assert_eq!(
        begin(&[a_and_down(), Input::new()])
            .run(Some(1))
            .report
            .frames,
        1
    );
}

#[test]
fn given_a_log_when_its_length_is_asked_then_it_is_the_number_of_frame_lines() {
    assert_eq!(begin(&[Input::new(); 3]).log_frames(), 3);
}

#[test]
fn given_a_log_when_replayed_in_one_run_or_two_then_the_reports_match() {
    let mut whole = begin(&[a_and_down(), Input::new(), a_and_down()]);
    let mut halves = begin(&[a_and_down(), Input::new(), a_and_down()]);
    halves.run(Some(1));
    assert_eq!(whole.run(None).report, halves.run(Some(2)).report);
}

#[test]
fn given_the_same_log_when_replayed_twice_then_the_reports_are_identical() {
    assert_eq!(
        begin(&[a_and_down(), Input::new()]).run(None),
        begin(&[a_and_down(), Input::new()]).run(None)
    );
}

#[test]
fn given_a_log_cut_off_mid_line_when_replayed_then_the_prefix_runs_and_the_drop_is_reported() {
    let rom = keypad_rom();
    let text = log_text(&rom, &[a_and_down()]) + "f=2 b=";
    let outcome = replay(rom, &text, None, None).expect("a prefix replays");
    assert_eq!(
        (outcome.report.frames, outcome.dropped_partial_line),
        (1, true)
    );
}

#[test]
fn given_a_whole_log_when_replayed_then_no_drop_is_reported() {
    let rom = keypad_rom();
    let text = log_text(&rom, &[a_and_down()]);
    assert!(!replay(rom, &text, None, None).unwrap().dropped_partial_line);
}

// ---- Refusals ----

#[test]
fn given_a_log_recorded_for_another_rom_when_replayed_then_the_rom_is_named() {
    let rom = keypad_rom();
    let crc = rom_crc(&rom);
    let header = LogHeader {
        rom_crc: crc ^ 1,
        ..header_for(&rom)
    };
    let text = encode_header(&header) + &encode_frame(1, &Input::new());
    assert_eq!(
        mismatch_of(replay(rom, &text, None, None)),
        Some(MismatchError::Rom {
            log: crc ^ 1,
            actual: crc
        })
    );
}

#[test]
fn given_a_log_that_began_with_a_save_when_replayed_with_other_save_bytes_then_the_hash_is_refused()
{
    let rom = keypad_rom();
    let blank = vec![0xFF; SaveType::Eeprom8K.len()];
    let mut changed = blank.clone();
    changed[0] = 0;
    let header = LogHeader {
        save_type: SaveType::Eeprom8K,
        save_hash: input_log::save_hash(&blank),
        ..header_for(&rom)
    };
    let text = encode_header(&header);
    assert_eq!(
        mismatch_of(replay(rom, &text, None, Some(changed.clone()))),
        Some(MismatchError::SaveHash {
            log: input_log::save_hash(&blank),
            actual: input_log::save_hash(&changed)
        })
    );
}

#[test]
fn given_a_log_that_began_with_a_save_when_replayed_with_the_same_bytes_then_it_runs() {
    let rom = keypad_rom();
    let blank = vec![0xFF; SaveType::Eeprom8K.len()];
    let header = LogHeader {
        save_type: SaveType::Eeprom8K,
        save_hash: input_log::save_hash(&blank),
        ..header_for(&rom)
    };
    let text = encode_header(&header) + &encode_frame(1, &Input::new());
    assert_eq!(
        replay(rom, &text, None, Some(blank)).unwrap().report.frames,
        1
    );
}

#[test]
fn given_a_log_that_began_with_a_save_when_replayed_with_no_file_then_the_blank_chip_matches() {
    let rom = keypad_rom();
    let blank = vec![0xFF; SaveType::Eeprom8K.len()];
    let header = LogHeader {
        save_type: SaveType::Eeprom8K,
        save_hash: input_log::save_hash(&blank),
        ..header_for(&rom)
    };
    assert!(replay(rom, &encode_header(&header), None, None).is_ok());
}

#[test]
fn given_a_save_file_of_the_wrong_size_when_replayed_then_the_save_is_refused() {
    let rom = keypad_rom();
    let header = LogHeader {
        save_type: SaveType::Eeprom8K,
        ..header_for(&rom)
    };
    assert!(matches!(
        replay(rom, &encode_header(&header), None, Some(vec![0; 3])),
        Err(RunnerError::Save(_))
    ));
}

#[test]
fn given_a_malformed_frame_when_replayed_then_the_log_error_names_its_line() {
    let rom = keypad_rom();
    let text = log_text(&rom, &[Input::new()]) + "f=3\n";
    assert_eq!(
        log_error_of(replay(rom, &text, None, None)),
        Some((
            3,
            LogErrorKind::FrameOutOfOrder {
                expected: 2,
                found: 3
            }
        ))
    );
}

#[test]
fn given_a_text_that_is_no_log_when_replayed_then_the_header_is_missing() {
    assert_eq!(
        log_error_of(replay(keypad_rom(), "hello\n", None, None)),
        Some((1, LogErrorKind::MissingHeader))
    );
}

#[test]
fn given_a_rom_the_machine_refuses_when_replayed_then_the_rom_error_comes_first() {
    let rom = keypad_rom();
    let text = log_text(&rom, &[Input::new()]);
    assert!(matches!(
        replay(vec![0; 16], &text, None, None),
        Err(RunnerError::Rom(_))
    ));
}

#[test]
fn given_the_refusals_when_displayed_then_each_says_what_was_refused() {
    let rom = keypad_rom();
    let log_error = replay(rom.clone(), "hello\n", None, None).unwrap_err();
    let header = LogHeader {
        rom_crc: rom_crc(&rom) ^ 1,
        ..header_for(&rom)
    };
    let mismatch = replay(rom, &encode_header(&header), None, None).unwrap_err();
    assert_eq!(
        (
            log_error
                .to_string()
                .starts_with("the input log was refused: input log line 1"),
            mismatch
                .to_string()
                .starts_with("the input log does not fit: the log is for a ROM"),
        ),
        (true, true)
    );
}

#[test]
fn given_the_refusals_when_their_source_is_asked_then_it_is_the_underlying_error() {
    use std::error::Error as _;
    let log_error = replay(keypad_rom(), "hello\n", None, None).unwrap_err();
    assert!(log_error.source().is_some());
}

// ---- The command line ----

#[test]
fn given_replay_with_a_rom_and_a_log_when_parsed_then_frames_and_save_are_unset() {
    let parsed = args(&["g.nds", "run.ninput"]).expect("valid");
    assert_eq!(
        (
            parsed.rom().to_path_buf(),
            parsed.log().to_path_buf(),
            parsed.frames(),
            parsed.save()
        ),
        (
            PathBuf::from("g.nds"),
            PathBuf::from("run.ninput"),
            None,
            None
        )
    );
}

#[test]
fn given_replay_with_frames_and_save_when_parsed_then_both_are_kept() {
    let parsed = args(&["g.nds", "l", "--frames", "9", "--save", "s.sav"]).expect("valid");
    assert_eq!(
        (parsed.frames(), parsed.save().map(Path::to_path_buf)),
        (Some(9), Some(PathBuf::from("s.sav")))
    );
}

#[test]
fn given_a_malformed_replay_command_when_parsed_then_each_case_is_a_usage_error() {
    let failures = usage_failures(&[
        &[],
        &["g.nds"],
        &["-g.nds", "l"],
        &["g.nds", "-l"],
        &["g.nds", "l", "--frames"],
        &["g.nds", "l", "--frames", "x"],
        &["g.nds", "l", "--frames", "1", "--frames", "2"],
        &["g.nds", "l", "--save"],
        &["g.nds", "l", "--save", "a", "--save", "b"],
        &["g.nds", "l", "extra"],
    ]);
    assert_eq!(failures, Vec::<Vec<String>>::new());
}

#[test]
fn given_replay_as_the_first_word_when_the_command_is_parsed_then_it_is_a_replay() {
    let command = parse_command(["replay", "g.nds", "l"].map(OsString::from));
    assert!(matches!(command, Ok(Command::Replay(_))));
}

#[test]
fn given_a_save_mismatch_when_displayed_then_the_message_says_how_to_pass_the_old_save() {
    let rom = keypad_rom();
    let blank = vec![0xFF; SaveType::Eeprom8K.len()];
    let header = LogHeader {
        save_type: SaveType::Eeprom8K,
        save_hash: input_log::save_hash(&blank),
        ..header_for(&rom)
    };
    let mut changed = blank;
    changed[0] = 0;
    let message = replay(rom, &encode_header(&header), None, Some(changed))
        .unwrap_err()
        .to_string();
    assert!(message.contains("pass the save as it was before recording with --save"));
}

#[test]
fn given_a_rom_mismatch_when_displayed_then_the_message_has_no_save_hint() {
    let rom = keypad_rom();
    let header = LogHeader {
        rom_crc: rom_crc(&rom) ^ 1,
        ..header_for(&rom)
    };
    let message = replay(rom, &encode_header(&header), None, None)
        .unwrap_err()
        .to_string();
    assert!(!message.contains("--save"));
}
