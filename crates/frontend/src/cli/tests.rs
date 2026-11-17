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
