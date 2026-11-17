// SPDX-License-Identifier: Apache-2.0
//! The save file's flush policy and its file IO. The policy is pure; the IO runs in the temp
//! directory under names that are not `.sav`.

use super::*;
use ninge_core::cart::SaveType;

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ninge-savefile-{}-{name}", std::process::id()))
}

fn fresh(name: &str) -> PathBuf {
    let path = temp(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// Ticks `n` frames with the counter at `generation`; the ticks that asked for a write.
fn ticks(flusher: &mut SaveFlusher, generation: u64, n: u32) -> Vec<u32> {
    (1..=n)
        .filter(|_| flusher.tick(generation))
        .collect::<Vec<_>>()
}

#[test]
fn given_an_unchanged_memory_when_frames_go_by_then_no_write_is_asked_for() {
    let mut flusher = SaveFlusher::new(0);
    assert_eq!(ticks(&mut flusher, 0, 200), Vec::<u32>::new());
}

#[test]
fn given_a_changed_memory_when_it_holds_still_then_a_write_is_asked_for_after_the_quiet_frames() {
    let mut flusher = SaveFlusher::new(0);
    // The first tick sees the change; the quiet count then runs to QUIET_FRAMES.
    let asked = (0..=QUIET_FRAMES + 1)
        .map(|_| flusher.tick(3))
        .position(|ask| ask);
    assert_eq!(asked, Some(QUIET_FRAMES as usize));
}

#[test]
fn given_a_memory_still_changing_when_frames_go_by_then_no_write_is_asked_for() {
    let mut flusher = SaveFlusher::new(0);
    let asked = (1..=500u64).filter(|&g| flusher.tick(g)).count();
    assert_eq!(asked, 0);
}

#[test]
fn given_a_write_asked_for_when_it_is_done_then_the_counter_is_current_and_quiet_again() {
    let mut flusher = SaveFlusher::new(0);
    ticks(&mut flusher, 5, QUIET_FRAMES + 1);
    flusher.wrote(5);
    assert_eq!(
        (flusher.pending(5), ticks(&mut flusher, 5, 200)),
        (false, Vec::<u32>::new())
    );
}

#[test]
fn given_a_failed_write_when_frames_go_by_then_the_next_try_is_a_quiet_spell_later() {
    let mut flusher = SaveFlusher::new(0);
    ticks(&mut flusher, 5, QUIET_FRAMES + 1);
    flusher.failed();
    let asked: Vec<u32> = ticks(&mut flusher, 5, QUIET_FRAMES);
    assert_eq!(asked, vec![QUIET_FRAMES]);
}

#[test]
fn given_a_counter_that_moved_when_asked_if_pending_then_it_is_pending() {
    assert!(SaveFlusher::new(2).pending(3));
}

#[test]
fn given_a_rom_path_when_the_default_save_is_named_then_it_has_the_extension_sav() {
    assert_eq!(
        default_path(Path::new("games/one.nds")),
        PathBuf::from("games/one.sav")
    );
}

#[test]
fn given_no_file_when_read_then_it_is_none() {
    assert!(matches!(read_optional(&fresh("absent.bin")), Ok(None)));
}

#[test]
fn given_a_directory_when_read_then_it_is_an_error() {
    assert!(read_optional(&std::env::temp_dir()).is_err());
}

#[test]
fn given_bytes_when_written_atomically_then_the_file_has_them_and_no_temp_file_stays() {
    let path = fresh("atomic.bin");
    write_atomic(&path, &[4, 5, 6]).expect("temp dir is writable");
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    assert_eq!(
        (
            std::fs::read(&path).expect("written"),
            Path::new(&tmp).exists()
        ),
        (vec![4, 5, 6], false)
    );
}

#[test]
fn given_a_missing_directory_when_written_then_it_is_an_error() {
    assert!(write_atomic(Path::new("/nonexistent/ninge/x.bin"), &[1]).is_err());
}

/// A booted machine with a 512-byte EEPROM fitted, whose save memory a state load then moves.
fn machine_with_changed_save() -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(std::fs::read(testrom()).expect("smoke ROM is built"))
        .expect("valid header");
    machine
        .insert_save(SaveType::Eeprom512, vec![0xCD; 512])
        .expect("512 bytes fit");
    machine
}

fn testrom() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testroms/build/smoke.nds")
}

/// `n` frames go by with the machine as it is.
fn frames(file: &mut SaveFile, machine: &Machine, n: u32) {
    (0..n).for_each(|_| file.after_frame(machine));
}

/// Moves the machine's save generation, as a savestate load does.
fn move_generation(machine: &mut Machine) {
    let state = machine.save_state();
    machine.load_state(&state).expect("same ROM");
}

#[test]
fn given_a_machine_whose_save_is_unchanged_when_frames_go_by_then_no_file_is_written() {
    let machine = machine_with_changed_save();
    let path = fresh("unchanged.bin");
    let mut file = SaveFile::new(path.clone(), &machine);
    frames(&mut file, &machine, 200);
    assert!(!path.exists());
}

#[test]
fn given_a_changed_save_when_the_quiet_frames_pass_then_the_file_is_written() {
    let mut machine = machine_with_changed_save();
    let path = fresh("changed.bin");
    let mut file = SaveFile::new(path.clone(), &machine);
    move_generation(&mut machine);
    frames(&mut file, &machine, QUIET_FRAMES + 2);
    assert_eq!(std::fs::read(&path).expect("written"), vec![0xCD; 512]);
}

#[test]
fn given_a_changed_save_when_the_program_exits_then_finish_writes_it_at_once() {
    let mut machine = machine_with_changed_save();
    let path = fresh("exit.bin");
    let mut file = SaveFile::new(path.clone(), &machine);
    move_generation(&mut machine);
    file.finish(&machine).expect("writable");
    assert_eq!(std::fs::read(&path).expect("written"), vec![0xCD; 512]);
}

#[test]
fn given_an_unchanged_save_when_the_program_exits_then_finish_writes_nothing() {
    let machine = machine_with_changed_save();
    let path = fresh("quiet-exit.bin");
    let mut file = SaveFile::new(path.clone(), &machine);
    file.finish(&machine).expect("nothing to do");
    assert!(!path.exists());
}

#[test]
fn given_an_unwritable_path_when_finishing_then_the_error_is_returned() {
    let mut machine = machine_with_changed_save();
    let mut file = SaveFile::new(PathBuf::from("/nonexistent/ninge/save.bin"), &machine);
    move_generation(&mut machine);
    assert!(file.finish(&machine).is_err());
}

#[test]
fn given_an_unwritable_path_when_frames_go_by_then_the_failure_is_kept_for_the_exit_flush() {
    let mut machine = machine_with_changed_save();
    let mut file = SaveFile::new(PathBuf::from("/nonexistent/ninge/save.bin"), &machine);
    move_generation(&mut machine);
    frames(&mut file, &machine, QUIET_FRAMES + 2);
    assert!(file.finish(&machine).is_err());
}

#[test]
fn given_a_save_file_when_its_path_is_asked_then_it_is_the_one_given() {
    let machine = machine_with_changed_save();
    let file = SaveFile::new(PathBuf::from("x.bin"), &machine);
    assert_eq!(file.path(), Path::new("x.bin"));
}

#[test]
fn given_a_machine_when_described_then_the_chip_and_the_file_are_named() {
    let machine = machine_with_changed_save();
    let file = SaveFile::new(PathBuf::from("x.bin"), &machine);
    assert_eq!(file.describe(&machine), "save chip eeprom-512, file x.bin");
}

#[test]
fn given_an_undecided_chip_with_a_moved_counter_when_finishing_then_no_empty_file_is_written() {
    let mut machine = Machine::new();
    machine
        .load_rom(std::fs::read(testrom()).expect("smoke ROM is built"))
        .expect("valid header");
    machine
        .insert_save(SaveType::Auto, Vec::new())
        .expect("auto");
    let path = fresh("undecided.bin");
    let mut file = SaveFile::new(path.clone(), &machine);
    move_generation(&mut machine);
    file.finish(&machine).expect("nothing to write");
    assert!(!path.exists());
}

#[test]
fn given_an_undecided_chip_with_a_moved_counter_when_frames_go_by_then_no_empty_file_is_written() {
    let mut machine = Machine::new();
    machine
        .load_rom(std::fs::read(testrom()).expect("smoke ROM is built"))
        .expect("valid header");
    machine
        .insert_save(SaveType::Auto, Vec::new())
        .expect("auto");
    let path = fresh("undecided-frames.bin");
    let mut file = SaveFile::new(path.clone(), &machine);
    move_generation(&mut machine);
    frames(&mut file, &machine, QUIET_FRAMES + 2);
    assert!(!path.exists());
}
