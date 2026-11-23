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
    let error = load(PathBuf::from("/nonexistent/ninge.nds"), &Options::default())
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
    let error = load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
        &Options::default(),
    )
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

fn smoke_rom() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testroms/build/smoke.nds")
}

fn temp_save(name: &str, len: usize) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ninge-source-{}-{name}", std::process::id()));
    std::fs::write(&path, vec![0x77; len]).expect("temp dir is writable");
    path
}

fn options(save: PathBuf, save_type: Option<ninge_core::cart::SaveType>) -> Options {
    Options {
        rom: Some(smoke_rom()),
        save: Some(save),
        save_type,
    }
}

fn machine_of(source: Source) -> Box<Machine> {
    match source {
        Source::Machine(machine) => machine,
        Source::Pattern => unreachable!("a ROM was given"),
    }
}

#[test]
fn given_a_save_file_of_a_chip_size_when_loading_then_the_machine_has_that_chip_and_data() {
    let path = temp_save("fits.bin", 8192);
    let (source, _) = load(smoke_rom(), &options(path, None)).expect("loads");
    let machine = machine_of(source);
    assert_eq!(
        (machine.save_type(), machine.save_data().to_vec()),
        (ninge_core::cart::SaveType::Eeprom8K, vec![0x77; 8192])
    );
}

#[test]
fn given_a_save_type_when_loading_then_it_wins_over_the_files_size_default() {
    let path = temp_save("fram.bin", 8192);
    let opts = options(path, Some(ninge_core::cart::SaveType::Fram8K));
    let (source, _) = load(smoke_rom(), &opts).expect("loads");
    assert_eq!(
        machine_of(source).save_type(),
        ninge_core::cart::SaveType::Fram8K
    );
}

#[test]
fn given_no_save_file_and_no_type_when_loading_then_the_chip_is_left_to_detection() {
    let path = std::env::temp_dir().join(format!("ninge-source-{}-none.bin", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let (source, _) = load(smoke_rom(), &options(path, None)).expect("loads");
    assert_eq!(
        machine_of(source).save_type(),
        ninge_core::cart::SaveType::Auto
    );
}

#[test]
fn given_a_save_file_of_no_chip_size_when_loading_then_it_is_refused_with_exit_4() {
    let path = temp_save("odd.bin", 1000);
    let error = load(smoke_rom(), &options(path, None))
        .err()
        .expect("refused");
    assert_eq!(
        (
            error.exit_code(),
            error.to_string().starts_with("the save was refused: ")
        ),
        (4, true)
    );
}

#[test]
fn given_a_save_type_and_a_file_of_another_size_when_loading_then_it_is_refused_with_exit_4() {
    let path = temp_save("mismatch.bin", 8192);
    let opts = options(path, Some(ninge_core::cart::SaveType::Flash256K));
    let error = load(smoke_rom(), &opts).err().expect("refused");
    assert_eq!(error.exit_code(), 4);
}

#[test]
fn given_a_save_path_that_is_a_directory_when_loading_then_it_is_an_io_error_naming_it() {
    let dir = std::env::temp_dir();
    let error = load(smoke_rom(), &options(dir.clone(), None))
        .err()
        .expect("fails");
    assert_eq!(
        (
            error.exit_code(),
            error.to_string().contains(&dir.display().to_string())
        ),
        (1, true)
    );
}

#[test]
fn given_no_rom_when_starting_then_the_pattern_shows_and_there_is_no_save_file() {
    let (source, save) = start(&Options::default()).expect("starts");
    assert!(matches!(source, Source::Pattern) && save.is_none());
}

#[test]
fn given_a_rom_when_starting_then_there_is_a_machine_and_a_save_file() {
    let (source, save) = start(&options(temp_save("start.bin", 512), None)).expect("starts");
    assert!(matches!(source, Source::Machine(_)) && save.is_some());
}
