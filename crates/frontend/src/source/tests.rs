// SPDX-License-Identifier: Apache-2.0
//! Tests of `source`.

use super::*;
use crate::cli::parse_args;

#[test]
fn given_empty_bytes_when_booting_then_the_header_is_refused() {
    assert!(boot(Vec::new()).is_err());
}

#[test]
fn given_missing_file_when_loading_then_it_is_an_io_error_naming_the_path() {
    let error = load(PathBuf::from("/nonexistent/ninge.nds"))
        .err()
        .expect("missing file");
    assert_eq!(
        (
            error.exit_code(),
            error
                .to_string()
                .starts_with("cannot read /nonexistent/ninge.nds: ")
        ),
        (1, true)
    );
}

#[test]
fn given_a_file_that_is_not_a_rom_when_loading_then_it_is_refused_with_exit_3() {
    let error = load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .err()
        .expect("not a ROM");
    assert_eq!(
        (
            error.exit_code(),
            error.to_string().starts_with("ROM refused: ")
        ),
        (3, true)
    );
}

#[test]
fn given_bad_arguments_when_converted_then_exit_code_is_2() {
    let usage = parse_args([std::ffi::OsString::from("-x")]).unwrap_err();
    assert_eq!(StartError::Usage(usage).exit_code(), 2);
}
