// SPDX-License-Identifier: Apache-2.0
//! Tests of `the crate root`.

use super::*;
use std::error::Error as _;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

fn parse(args: &[&str]) -> Result<Args, RunnerError> {
    parse_args(args.iter().map(OsString::from))
}

fn missing_rom_error() -> RunnerError {
    load_rom_file(Path::new("/nonexistent/ninge/none.nds")).expect_err("path does not exist")
}

fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ninge-runner-{}-{name}", std::process::id()));
    std::fs::write(&path, bytes).expect("temp dir is writable");
    path
}

#[test]
fn given_only_a_rom_path_when_parsing_args_then_frames_default_to_one() {
    let args = parse(&["game.nds"]).expect("valid args");
    assert_eq!(
        (args.rom().to_path_buf(), args.frames()),
        (PathBuf::from("game.nds"), 1)
    );
}

#[test]
fn given_frames_flag_when_parsing_args_then_frames_is_the_given_count() {
    let args = parse(&["game.nds", "--frames", "5"]).expect("valid args");
    assert_eq!(
        (args.rom().to_path_buf(), args.frames()),
        (PathBuf::from("game.nds"), 5)
    );
}

#[test]
fn given_no_arguments_when_parsing_args_then_usage_error() {
    assert!(matches!(parse(&[]), Err(RunnerError::Usage { .. })));
}

#[test]
fn given_non_numeric_frames_when_parsing_args_then_usage_error() {
    let result = parse(&["game.nds", "--frames", "many"]);
    assert!(matches!(result, Err(RunnerError::Usage { .. })));
}

#[test]
fn given_frames_flag_without_value_when_parsing_args_then_usage_error() {
    let result = parse(&["game.nds", "--frames"]);
    assert!(matches!(result, Err(RunnerError::Usage { .. })));
}

#[test]
fn given_extra_argument_after_frames_when_parsing_args_then_usage_error() {
    let result = parse(&["rom", "--frames", "2", "extra"]);
    assert!(matches!(result, Err(RunnerError::Usage { .. })));
}

#[test]
fn given_repeated_frames_flag_when_parsing_args_then_usage_error() {
    let result = parse(&["rom", "--frames", "2", "--frames", "3"]);
    assert!(matches!(result, Err(RunnerError::Usage { .. })));
}

#[test]
fn given_flag_in_rom_position_when_parsing_args_then_usage_error() {
    let result = parse(&["--help"]);
    assert!(matches!(result, Err(RunnerError::Usage { .. })));
}

#[cfg(unix)]
#[test]
fn given_non_utf8_rom_path_when_parsing_args_then_rom_path_is_kept() {
    use std::os::unix::ffi::OsStringExt;
    let raw = OsString::from_vec(vec![b'r', 0xFF, 0xFE, b'.', b'n', b'd', b's']);
    let args = parse_args([raw.clone()]).expect("valid args");
    assert_eq!(args.rom(), Path::new(&raw));
}

#[test]
fn given_no_pixels_when_hashing_then_hash_is_the_fnv1a_offset_basis() {
    assert_eq!(frame_hash(&[]), 0xcbf2_9ce4_8422_2325);
}

#[test]
fn given_one_pixel_when_hashing_then_hash_is_fnv1a_over_its_little_endian_bytes() {
    // FNV-1a 64 over bytes 01 02 03 04 (pixel 0x0403_0201, little-endian):
    // h = 0xcbf29ce484222325; for each byte b: h ^= b; h = h.wrapping_mul(0x100000001b3).
    // Computed independently with a python3 snippet (big-int arithmetic masked to 64 bits).
    assert_eq!(frame_hash(&[0x0403_0201]), 0xbe7a_5e77_5165_785d);
}

#[test]
fn given_one_black_pixel_when_hashing_then_hash_is_fnv1a_over_four_zero_bytes() {
    // Same python3 computation over 00 00 00 00.
    assert_eq!(frame_hash(&[0]), 0x4d25_767f_9dce_13f5);
}

#[test]
fn given_a_run_report_when_displayed_then_it_is_one_key_value_line() {
    let report = RunReport {
        frames: 3,
        top_hash: 0xab,
        bottom_hash: 0x1234_5678_9abc_def0,
        audio_hash: None,
    };
    assert_eq!(
        report.to_string(),
        "frames=3 top=00000000000000ab bottom=123456789abcdef0"
    );
}

#[test]
fn given_usage_error_when_displayed_then_message_is_not_empty() {
    let err = parse(&[]).expect_err("no rom path");
    assert!(!err.to_string().is_empty());
}

#[test]
fn given_io_error_when_displayed_then_message_names_the_path() {
    assert!(
        missing_rom_error()
            .to_string()
            .contains("/nonexistent/ninge/none.nds")
    );
}

#[test]
fn given_io_error_when_asking_for_source_then_underlying_error_is_present() {
    assert!(missing_rom_error().source().is_some());
}

#[test]
fn given_usage_error_when_asking_for_source_then_there_is_none() {
    let err = parse(&[]).expect_err("no rom path");
    assert!(err.source().is_none());
}

#[test]
fn given_missing_file_when_loading_rom_then_io_error() {
    assert!(matches!(missing_rom_error(), RunnerError::Io { .. }));
}

#[test]
fn given_existing_file_when_loading_rom_then_bytes_are_returned_verbatim() {
    let path = write_temp("verbatim.bin", &[9, 8, 7]);
    let loaded = load_rom_file(&path).expect("file exists");
    std::fs::remove_file(&path).expect("cleanup");
    assert_eq!(loaded, vec![9, 8, 7]);
}

fn command(args: &[&str]) -> Result<Command, RunnerError> {
    parse_command(args.iter().map(OsString::from))
}

/// A short description of a parsed command or of why it was refused.
fn describe(parsed: Result<Command, RunnerError>) -> String {
    match parsed {
        Ok(Command::Run(args)) => format!("run {} {}", args.rom().display(), args.frames()),
        Ok(Command::Replay(args)) => {
            format!("replay {} {}", args.rom().display(), args.log().display())
        }
        Ok(Command::CpuSuite(dir)) => format!("cpu-suite {}", dir.display()),
        Err(RunnerError::Usage(message)) => {
            format!("usage: {}", message.lines().next().unwrap_or(""))
        }
        Err(other) => format!("other: {other}"),
    }
}

#[test]
fn given_cpu_suite_and_a_directory_when_parsing_a_command_then_it_selects_the_suite() {
    assert_eq!(
        describe(command(&["cpu-suite", "some/dir"])),
        "cpu-suite some/dir"
    );
}

#[test]
fn given_replay_a_rom_and_a_log_when_parsing_a_command_then_it_selects_a_replay() {
    assert_eq!(
        describe(command(&["replay", "game.nds", "run.ninput"])),
        "replay game.nds run.ninput"
    );
}

#[test]
fn given_a_dot_slash_rom_named_replay_when_parsing_a_command_then_it_is_still_a_run() {
    assert_eq!(describe(command(&["./replay"])), "run ./replay 1");
}

#[test]
fn given_a_rom_path_when_parsing_a_command_then_it_selects_a_run() {
    assert_eq!(
        describe(command(&["game.nds", "--frames", "4"])),
        "run game.nds 4"
    );
}

#[test]
fn given_cpu_suite_without_a_directory_when_parsing_a_command_then_it_is_a_usage_error() {
    assert_eq!(
        describe(command(&["cpu-suite"])),
        "usage: missing suite directory"
    );
}

#[test]
fn given_cpu_suite_with_an_extra_argument_when_parsing_a_command_then_it_is_a_usage_error() {
    assert_eq!(
        describe(command(&["cpu-suite", "dir", "more"])),
        "usage: unexpected argument more"
    );
}

#[test]
fn given_no_arguments_when_parsing_a_command_then_it_is_the_missing_rom_usage_error() {
    assert_eq!(describe(command(&[])), "usage: missing ROM path");
}

#[test]
fn given_cpu_suite_with_a_flag_as_the_directory_when_parsing_a_command_then_it_is_a_usage_error() {
    assert_eq!(
        describe(command(&["cpu-suite", "--help"])),
        "usage: expected a directory, found --help"
    );
}

#[test]
fn given_a_rom_named_with_a_dot_slash_cpu_suite_when_parsing_a_command_then_it_is_a_rom() {
    assert_eq!(describe(command(&["./cpu-suite"])), "run ./cpu-suite 1");
}

#[test]
fn given_a_suite_error_when_converted_then_it_displays_and_exposes_its_source() {
    let error = RunnerError::from(cpu_suite::SuiteError::Format {
        offset: 3,
        reason: "bad".to_string(),
    });
    assert_eq!(
        (error.to_string(), error.source().is_some()),
        ("bad suite data at byte 3: bad".to_string(), true)
    );
}

// ------------------------------------------------------------------------------- save files

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ninge-runner-save-{}-{name}", std::process::id()))
}

/// A path under the temp dir that does not exist yet.
fn fresh_path(name: &str) -> PathBuf {
    let path = temp_path(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn smoke_rom() -> Vec<u8> {
    load_rom_file(&testrom_path("smoke")).expect("smoke ROM is built")
}

#[test]
fn given_save_flags_when_parsing_args_then_the_setup_has_the_path_and_the_type() {
    let args = parse(&["g.nds", "--save", "x.bin", "--save-type", "flash-512k"]).expect("valid");
    assert_eq!(
        args.save_setup(),
        SaveSetup {
            path: PathBuf::from("x.bin"),
            kind: Some(SaveType::Flash512K)
        }
    );
}

#[test]
fn given_no_save_flags_when_parsing_args_then_the_file_sits_beside_the_rom_and_the_type_is_open() {
    let args = parse(&["dir/g.nds"]).expect("valid");
    assert_eq!(
        args.save_setup(),
        SaveSetup {
            path: PathBuf::from("dir/g.sav"),
            kind: None
        }
    );
}

#[test]
fn given_an_unknown_save_type_when_parsing_args_then_the_usage_error_lists_the_types() {
    let message = parse(&["g.nds", "--save-type", "flash-3m"])
        .expect_err("unknown type")
        .to_string();
    assert!(message.contains("flash-3m") && message.contains("eeprom-8k"));
}

#[test]
fn given_a_save_flag_without_a_value_when_parsing_args_then_usage_error() {
    assert!(matches!(
        parse(&["g.nds", "--save"]),
        Err(RunnerError::Usage(_))
    ));
}

#[test]
fn given_a_save_type_flag_without_a_value_when_parsing_args_then_usage_error() {
    assert!(matches!(
        parse(&["g.nds", "--save-type"]),
        Err(RunnerError::Usage(_))
    ));
}

#[test]
fn given_a_repeated_save_flag_when_parsing_args_then_usage_error() {
    assert!(matches!(
        parse(&["g.nds", "--save", "a", "--save", "b"]),
        Err(RunnerError::Usage(_))
    ));
}

#[test]
fn given_a_rom_path_without_an_extension_when_the_default_save_is_named_then_sav_is_added() {
    assert_eq!(
        default_save_path(Path::new("dir/game")),
        PathBuf::from("dir/game.sav")
    );
}

#[test]
fn given_no_file_when_a_save_is_read_then_it_is_none_not_an_error() {
    assert!(matches!(
        read_save_file(&fresh_path("absent.bin")),
        Ok(None)
    ));
}

#[test]
fn given_a_file_when_a_save_is_read_then_the_bytes_come_back() {
    let path = write_temp("present.bin", &[1, 2, 3]);
    assert_eq!(
        read_save_file(&path).expect("readable"),
        Some(vec![1, 2, 3])
    );
}

#[test]
fn given_a_directory_when_a_save_is_read_then_it_is_an_io_error() {
    assert!(matches!(
        read_save_file(&std::env::temp_dir()),
        Err(RunnerError::Io { .. })
    ));
}

#[test]
fn given_bytes_when_a_save_is_written_then_the_file_has_them_and_no_temp_file_is_left() {
    let path = fresh_path("atomic.bin");
    write_save_file(&path, &[9, 8, 7]).expect("temp dir is writable");
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    assert_eq!(
        (
            std::fs::read(&path).expect("written"),
            Path::new(&temp).exists()
        ),
        (vec![9, 8, 7], false)
    );
}

#[test]
fn given_an_existing_file_when_a_save_is_written_then_it_is_replaced() {
    let path = write_temp("replace.bin", &[1, 1, 1, 1]);
    write_save_file(&path, &[2, 2]).expect("temp dir is writable");
    assert_eq!(std::fs::read(&path).expect("written"), vec![2, 2]);
}

#[test]
fn given_a_missing_directory_when_a_save_is_written_then_it_is_a_write_error_naming_the_path() {
    let error = write_save_file(Path::new("/nonexistent/ninge/save.bin"), &[1])
        .expect_err("no such directory");
    assert!(error.to_string().contains("save.bin"));
}

#[test]
fn given_a_write_error_when_displayed_then_it_says_cannot_write() {
    let error = write_save_file(Path::new("/nonexistent/ninge/save.bin"), &[1])
        .expect_err("no such directory");
    assert!(
        error
            .to_string()
            .starts_with("cannot write /nonexistent/ninge/save.bin: ")
    );
}

#[test]
fn given_a_write_error_when_asking_for_source_then_the_io_error_is_present() {
    let error = write_save_file(Path::new("/nonexistent/ninge/save.bin"), &[1])
        .expect_err("no such directory");
    assert!(error.source().is_some());
}

#[test]
fn given_a_refused_save_when_displayed_then_it_says_so_and_exposes_its_source() {
    let error = RunnerError::Save(SaveError::UnknownSize { len: 5 });
    assert_eq!(
        (
            error.to_string().starts_with("the save was refused: "),
            error.source().is_some()
        ),
        (true, true)
    );
}

#[test]
fn given_a_machine_whose_save_changed_when_flushed_then_the_file_is_written_once() {
    let mut machine = Machine::new();
    machine.load_rom(smoke_rom()).expect("smoke ROM");
    machine
        .insert_save(SaveType::Eeprom512, vec![0xAB; 512])
        .expect("512 bytes fit");
    let baseline = machine.save_generation();
    // A savestate load raises the generation: the host must treat the memory as changed.
    let state = machine.save_state();
    machine.load_state(&state).expect("same ROM");
    let path = fresh_path("flush.bin");
    let first = flush_save(&machine, baseline, &path).expect("writable");
    let again = flush_save(&machine, machine.save_generation(), &path).expect("writable");
    assert_eq!(
        (first, again, std::fs::read(&path).expect("written")),
        (true, false, vec![0xAB; 512])
    );
}

#[test]
fn given_an_undecided_chip_when_flushed_then_nothing_is_written() {
    let mut machine = Machine::new();
    machine.load_rom(smoke_rom()).expect("smoke ROM");
    machine
        .insert_save(SaveType::Auto, Vec::new())
        .expect("auto");
    let state = machine.save_state();
    machine.load_state(&state).expect("same ROM");
    let path = fresh_path("undecided.bin");
    assert_eq!(
        (flush_save(&machine, 0, &path).expect("ok"), path.exists()),
        (false, false)
    );
}

#[test]
fn given_a_save_file_of_a_chip_size_when_a_run_does_not_write_then_the_file_is_untouched() {
    let path = write_temp("untouched.bin", &[0x5A; 8192]);
    let setup = SaveSetup {
        path: path.clone(),
        kind: None,
    };
    run_with_save(smoke_rom(), 1, &setup).expect("run");
    assert_eq!(std::fs::read(&path).expect("kept"), vec![0x5A; 8192]);
}

#[test]
fn given_a_save_file_of_no_chip_size_when_run_then_the_save_is_refused() {
    let path = write_temp("oddsize.bin", &[0; 1000]);
    let setup = SaveSetup { path, kind: None };
    assert!(matches!(
        run_with_save(smoke_rom(), 1, &setup),
        Err(RunnerError::Save(SaveError::UnknownSize { len: 1000 }))
    ));
}

#[test]
fn given_a_save_type_and_a_file_of_another_size_when_run_then_the_save_is_refused() {
    let path = write_temp("mismatch.bin", &[0; 8192]);
    let setup = SaveSetup {
        path,
        kind: Some(SaveType::Flash256K),
    };
    assert!(matches!(
        run_with_save(smoke_rom(), 1, &setup),
        Err(RunnerError::Save(SaveError::BadLength { .. }))
    ));
}

#[test]
fn given_a_save_type_and_no_file_when_a_run_does_not_write_then_no_file_is_created() {
    let path = fresh_path("nocreate.bin");
    let setup = SaveSetup {
        path: path.clone(),
        kind: Some(SaveType::Eeprom8K),
    };
    run_with_save(smoke_rom(), 1, &setup).expect("run");
    assert!(!path.exists());
}

#[test]
fn given_a_save_outcome_when_displayed_then_it_names_the_chip_and_whether_the_file_was_written() {
    let written = SaveOutcome {
        kind: SaveType::Eeprom64K,
        written: true,
    };
    let kept = SaveOutcome {
        kind: SaveType::Auto,
        written: false,
    };
    assert_eq!(
        (written.to_string(), kept.to_string()),
        (
            "save chip eeprom-64k, file written".to_string(),
            "save chip auto".to_string()
        )
    );
}

#[test]
fn given_a_run_that_writes_nothing_when_it_ends_then_the_outcome_has_the_chip_and_no_write() {
    let path = fresh_path("outcome.bin");
    let setup = SaveSetup {
        path,
        kind: Some(SaveType::Eeprom8K),
    };
    let (_, outcome) = run_with_save(smoke_rom(), 1, &setup).expect("run");
    assert_eq!(
        outcome,
        SaveOutcome {
            kind: SaveType::Eeprom8K,
            written: false
        }
    );
}

// ----------------------------------------------------------------------------------------- audio

#[test]
fn given_a_report_with_an_audio_hash_when_displayed_then_the_line_has_a_fourth_field() {
    let report = RunReport {
        frames: 3,
        top_hash: 0xab,
        bottom_hash: 0xcd,
        audio_hash: Some(0x0123_4567_89ab_cdef),
    };
    assert_eq!(
        report.to_string(),
        "frames=3 top=00000000000000ab bottom=00000000000000cd audio=0123456789abcdef"
    );
}

#[test]
fn given_no_samples_when_hashing_the_audio_then_it_is_the_fnv1a_offset_basis() {
    assert_eq!(audio_hash(&[]), 0xcbf2_9ce4_8422_2325);
}

#[test]
fn given_one_zero_sample_when_hashing_the_audio_then_it_is_fnv1a_over_two_zero_bytes() {
    // Computed independently with python3: FNV-1a 64 over 00 00.
    assert_eq!(audio_hash(&[0]), 0x0832_8807_b4eb_6fed);
}

#[test]
fn given_two_samples_when_hashing_the_audio_then_it_is_fnv1a_over_their_little_endian_bytes() {
    // 1 and -1 are 01 00 and FF FF; the same python3 computation over 01 00 FF FF.
    assert_eq!(audio_hash(&[1, -1]), 0xab59_b177_460d_9ada);
}

#[test]
fn given_samples_in_two_parts_when_the_hash_is_updated_in_turn_then_it_equals_the_whole_hash() {
    let part = audio_hash_update(AUDIO_HASH_START, &[1]);
    assert_eq!(audio_hash_update(part, &[-1]), audio_hash(&[1, -1]));
}

#[test]
fn given_two_samples_when_written_as_a_wav_then_the_bytes_are_the_44_byte_header_and_the_data() {
    // RIFF, size 36 + 4, WAVE, "fmt " 16, PCM 1, 2 channels, 32728 Hz (7FD8h), byte rate 130912
    // (1FF60h), block align 4, 16 bits, "data", 4 bytes, then 01 00 FF FF.
    let expected: Vec<u8> = [
        b"RIFF".as_slice(),
        &[40, 0, 0, 0],
        b"WAVEfmt ",
        &[16, 0, 0, 0, 1, 0, 2, 0],
        &[0xD8, 0x7F, 0, 0],
        &[0x60, 0xFF, 0x01, 0],
        &[4, 0, 16, 0],
        b"data",
        &[4, 0, 0, 0],
        &[1, 0, 0xFF, 0xFF],
    ]
    .concat();
    assert_eq!(wav_bytes(&[1, -1]), expected);
}

#[test]
fn given_no_samples_when_written_as_a_wav_then_it_is_just_the_header() {
    assert_eq!(wav_bytes(&[]).len(), 44);
}

#[test]
fn given_the_audio_flag_when_parsing_args_then_audio_is_asked_for_and_there_is_no_wav() {
    let args = parse(&["rom", "--audio"]).expect("valid args");
    assert_eq!((args.audio(), args.wav()), (true, None));
}

#[test]
fn given_the_wav_flag_when_parsing_args_then_audio_is_implied_and_the_path_is_kept() {
    let args = parse(&["rom", "--wav", "out.wav"]).expect("valid args");
    assert_eq!(
        (args.audio(), args.wav()),
        (true, Some(Path::new("out.wav")))
    );
}

#[test]
fn given_no_audio_flags_when_parsing_args_then_audio_is_not_asked_for() {
    let args = parse(&["rom", "--frames", "2"]).expect("valid args");
    assert_eq!((args.audio(), args.wav()), (false, None));
}

#[test]
fn given_a_repeated_audio_flag_or_a_wav_without_a_path_when_parsing_then_usage_error() {
    let repeated = parse(&["rom", "--audio", "--audio"]);
    let bare = parse(&["rom", "--wav"]);
    assert!(
        matches!(repeated, Err(RunnerError::Usage(_)))
            && matches!(bare, Err(RunnerError::Usage(_)))
    );
}
