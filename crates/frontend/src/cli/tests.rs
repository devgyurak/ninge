// SPDX-License-Identifier: Apache-2.0
//! Tests of `cli`.

use super::*;

fn parse(args: &[&str]) -> Result<Options, UsageError> {
    parse_args(args.iter().map(OsString::from))
}

fn rom_only(path: &str) -> Options {
    Options {
        rom: Some(PathBuf::from(path)),
        ..Options::default()
    }
}

#[test]
fn given_no_arguments_when_parsing_then_there_is_no_rom() {
    assert_eq!(parse(&[]), Ok(Options::default()));
}

#[test]
fn given_one_path_when_parsing_then_it_is_the_rom() {
    assert_eq!(parse(&["a.nds"]), Ok(rom_only("a.nds")));
}

#[test]
fn given_two_paths_when_parsing_then_it_is_a_usage_error() {
    let message = parse(&["a.nds", "b.nds"]).unwrap_err().to_string();
    assert_eq!(message, format!("expected at most one ROM path\n{USAGE}"));
}

#[test]
fn given_an_option_when_parsing_then_it_is_a_usage_error() {
    let message = parse(&["-x"]).unwrap_err().to_string();
    assert_eq!(message, format!("unknown option -x\n{USAGE}"));
}

#[test]
fn given_save_flags_when_parsing_then_the_options_carry_the_path_and_the_type() {
    assert_eq!(
        parse(&["a.nds", "--save", "s.bin", "--save-type", "flash-512k"]),
        Ok(Options {
            rom: Some(PathBuf::from("a.nds")),
            save: Some(PathBuf::from("s.bin")),
            save_type: Some(SaveType::Flash512K),
            record: None,
            no_audio: false,
            focus_pause: false,
        })
    );
}

#[test]
fn given_the_flags_before_the_rom_when_parsing_then_the_rom_is_still_found() {
    assert_eq!(
        parse(&["--save-type", "none", "a.nds"]),
        Ok(Options {
            rom: Some(PathBuf::from("a.nds")),
            save: None,
            save_type: Some(SaveType::None),
            record: None,
            no_audio: false,
            focus_pause: false,
        })
    );
}

#[test]
fn given_a_save_flag_without_a_value_when_parsing_then_it_is_a_usage_error() {
    let message = parse(&["a.nds", "--save"]).unwrap_err().to_string();
    assert_eq!(message, format!("--save needs a path\n{USAGE}"));
}

#[test]
fn given_a_save_type_flag_without_a_value_when_parsing_then_it_is_a_usage_error() {
    let message = parse(&["a.nds", "--save-type"]).unwrap_err().to_string();
    assert_eq!(message, format!("--save-type needs a name\n{USAGE}"));
}

#[test]
fn given_an_unknown_save_type_when_parsing_then_the_error_lists_the_types() {
    let message = parse(&["a.nds", "--save-type", "flash-3m"])
        .unwrap_err()
        .to_string();
    assert!(message.contains("flash-3m") && message.contains("eeprom-8k"));
}

#[test]
fn given_a_save_flag_and_no_rom_when_parsing_then_it_is_a_usage_error() {
    let message = parse(&["--save", "s.bin"]).unwrap_err().to_string();
    assert_eq!(
        message,
        format!("--save and --save-type need a ROM\n{USAGE}")
    );
}

#[test]
fn given_a_repeated_save_flag_when_parsing_then_it_is_a_usage_error() {
    let message = parse(&["a.nds", "--save", "x", "--save", "y"])
        .unwrap_err()
        .to_string();
    assert_eq!(message, format!("unknown option --save\n{USAGE}"));
}

#[test]
fn given_no_audio_alone_when_parsing_then_it_is_set_and_there_is_no_rom() {
    assert_eq!(
        parse(&["--no-audio"]),
        Ok(Options {
            no_audio: true,
            ..Options::default()
        })
    );
}

#[test]
fn given_no_audio_with_a_rom_and_save_flags_when_parsing_then_all_of_them_are_kept() {
    assert_eq!(
        parse(&["a.nds", "--save", "s.bin", "--no-audio"]),
        Ok(Options {
            rom: Some(PathBuf::from("a.nds")),
            save: Some(PathBuf::from("s.bin")),
            no_audio: true,
            ..Options::default()
        })
    );
}

#[test]
fn given_no_audio_twice_when_parsing_then_it_is_a_usage_error() {
    let message = parse(&["--no-audio", "--no-audio"])
        .unwrap_err()
        .to_string();
    assert_eq!(message, format!("unknown option --no-audio\n{USAGE}"));
}

#[test]
fn given_no_arguments_when_parsing_then_audio_is_on() {
    assert!(!parse(&[]).unwrap().no_audio);
}

#[test]
fn given_no_arguments_when_parsing_then_the_run_does_not_pause_without_focus() {
    assert!(!parse(&[]).unwrap().focus_pause);
}

#[test]
fn given_focus_pause_alone_when_parsing_then_it_is_set_and_there_is_no_rom() {
    assert_eq!(
        parse(&["--focus-pause"]),
        Ok(Options {
            focus_pause: true,
            ..Options::default()
        })
    );
}

#[test]
fn given_focus_pause_after_a_rom_and_no_audio_when_parsing_then_all_of_them_are_kept() {
    assert_eq!(
        parse(&["--no-audio", "a.nds", "--focus-pause"]),
        Ok(Options {
            rom: Some(PathBuf::from("a.nds")),
            no_audio: true,
            focus_pause: true,
            ..Options::default()
        })
    );
}

#[test]
fn given_focus_pause_twice_when_parsing_then_it_is_a_usage_error() {
    let message = parse(&["--focus-pause", "--focus-pause"])
        .unwrap_err()
        .to_string();
    assert_eq!(message, format!("unknown option --focus-pause\n{USAGE}"));
}

#[test]
fn given_record_and_a_path_when_parsing_then_the_options_carry_it() {
    assert_eq!(
        parse(&["a.nds", "--record", "run.ninput"]),
        Ok(Options {
            record: Some(PathBuf::from("run.ninput")),
            ..rom_only("a.nds")
        })
    );
}

#[test]
fn given_a_bad_record_flag_when_parsing_then_each_case_is_a_usage_error() {
    let cases: [&[&str]; 3] = [
        &["a.nds", "--record"],
        &["a.nds", "--record", "a", "--record", "b"],
        &["--record", "run.ninput"],
    ];
    let failures: Vec<_> = cases.iter().filter(|case| parse(case).is_ok()).collect();
    assert_eq!(failures, Vec::<&&[&str]>::new());
}

#[test]
fn given_record_without_a_rom_when_parsing_then_the_message_says_it_needs_one() {
    let message = parse(&["--record", "run.ninput"]).unwrap_err().to_string();
    assert_eq!(message, format!("--record needs a ROM\n{USAGE}"));
}
