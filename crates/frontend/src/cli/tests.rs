// SPDX-License-Identifier: Apache-2.0
//! Tests of `cli`.

use super::*;

fn parse(args: &[&str]) -> Result<Option<PathBuf>, UsageError> {
    parse_args(args.iter().map(OsString::from))
}

#[test]
fn given_no_arguments_when_parsing_then_there_is_no_rom() {
    assert_eq!(parse(&[]), Ok(None));
}

#[test]
fn given_one_path_when_parsing_then_it_is_the_rom() {
    assert_eq!(parse(&["a.nds"]), Ok(Some(PathBuf::from("a.nds"))));
}

#[test]
fn given_two_paths_when_parsing_then_it_is_a_usage_error() {
    let message = parse(&["a.nds", "b.nds"]).unwrap_err().to_string();
    assert_eq!(
        message,
        "expected at most one ROM path\nusage: ninge [<rom>]"
    );
}

#[test]
fn given_an_option_when_parsing_then_it_is_a_usage_error() {
    let message = parse(&["-x"]).unwrap_err().to_string();
    assert_eq!(message, "unknown option -x\nusage: ninge [<rom>]");
}
