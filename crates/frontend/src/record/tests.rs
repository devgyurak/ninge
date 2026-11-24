// SPDX-License-Identifier: Apache-2.0
//! Tests of `record`: the log's text is checked by reading it back with the core's parser.

use super::*;
use crate::source::boot;
use ninge_core::cart::SaveType;
use ninge_core::input_log::{self, ParsedLog};
use ninge_core::{Buttons, Touch};
use std::sync::{Arc, Mutex};

/// A writer whose bytes the test can read after the recorder owns it.
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

impl Shared {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("not poisoned").clone()).expect("utf-8")
    }
}

impl Write for Shared {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("not poisoned")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A writer that takes `ok` writes and then fails every one, counting the attempts.
struct FailsAfter {
    ok: usize,
    attempts: Arc<Mutex<usize>>,
}

impl Write for FailsAfter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        *self.attempts.lock().expect("not poisoned") += 1;
        match self.ok.checked_sub(1) {
            Some(left) => {
                self.ok = left;
                Ok(bytes.len())
            }
            None => Err(io::Error::other("disk full")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn smoke() -> Machine {
    let rom = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testroms/build/smoke.nds"),
    )
    .expect("smoke ROM is built");
    boot(rom).expect("valid header")
}

fn begin(machine: &Machine) -> (Recorder, Shared) {
    let shared = Shared::default();
    let recorder = Recorder::begin(
        Box::new(shared.clone()),
        PathBuf::from("run.ninput"),
        machine,
    )
    .expect("the header is written");
    (recorder, shared)
}

fn parsed(shared: &Shared) -> ParsedLog {
    input_log::parse(&shared.text()).expect("a log the core reads")
}

/// Records each of `inputs` in order.
fn record_all(recorder: &mut Recorder, inputs: &[Input]) {
    inputs.iter().for_each(|input| recorder.record(input));
}

fn pressed() -> Input {
    Input::new()
        .with_buttons(Buttons::A | Buttons::START)
        .with_touch(Some(Touch { x: 12, y: 34 }))
        .with_hinge_closed(true)
        .with_mic(true)
        .with_wall_time(789_012_345)
}

#[test]
fn given_a_booted_machine_when_recording_begins_then_the_header_names_its_rom_and_an_empty_save() {
    let machine = smoke();
    let (_, shared) = begin(&machine);
    let crc = machine.header().expect("a header").crc;
    assert_eq!(
        shared.text(),
        format!("ninge-input 1 rom={crc:04x} save=none:cbf29ce484222325\n")
    );
}

#[test]
fn given_a_machine_with_a_save_when_recording_begins_then_the_header_hashes_its_memory() {
    let mut machine = smoke();
    machine
        .insert_save(SaveType::Eeprom512, vec![0x21; 512])
        .expect("512 bytes fit");
    let (_, shared) = begin(&machine);
    let header = parsed(&shared).header;
    assert_eq!(
        (header.save_type, header.save_hash),
        (SaveType::Eeprom512, input_log::save_hash(&[0x21; 512]))
    );
}

#[test]
fn given_inputs_when_recorded_then_the_log_parses_back_to_the_same_inputs() {
    let machine = smoke();
    let (mut recorder, shared) = begin(&machine);
    let inputs = [pressed(), Input::new(), Input::new().with_mic(true)];
    record_all(&mut recorder, &inputs);
    assert_eq!(parsed(&shared).frames, inputs.to_vec());
}

#[test]
fn given_a_recorded_log_when_checked_against_the_machine_then_its_header_fits() {
    let machine = smoke();
    let (_, shared) = begin(&machine);
    let crc = machine.header().expect("a header").crc;
    assert_eq!(
        parsed(&shared).header.check(
            crc,
            machine.save_type(),
            input_log::save_hash(machine.save_data())
        ),
        Ok(())
    );
}

#[test]
fn given_frames_recorded_when_counted_then_the_note_says_how_many_and_where() {
    let machine = smoke();
    let (mut recorder, _) = begin(&machine);
    recorder.record(&Input::new());
    recorder.record(&Input::new());
    assert_eq!(
        (recorder.frames(), recorder.note()),
        (2, "recorded 2 frames to run.ninput".to_string())
    );
}

#[test]
fn given_a_sink_that_fails_on_the_second_frame_when_recording_then_the_note_says_why() {
    let machine = smoke();
    // Two writes succeed: the header and the first frame.
    let attempts = Arc::new(Mutex::new(0));
    let sink = FailsAfter {
        ok: 2,
        attempts: attempts.clone(),
    };
    let mut recorder =
        Recorder::begin(Box::new(sink), PathBuf::from("run.ninput"), &machine).expect("header");
    recorder.record(&Input::new());
    recorder.record(&Input::new());
    assert_eq!(
        (recorder.frames(), recorder.note()),
        (
            1,
            "recording to run.ninput stopped after 1 frames: disk full".to_string()
        )
    );
}

#[test]
fn given_a_recording_that_stopped_when_more_frames_come_then_nothing_more_is_written() {
    let machine = smoke();
    let attempts = Arc::new(Mutex::new(0));
    let sink = FailsAfter {
        ok: 1,
        attempts: attempts.clone(),
    };
    let mut recorder =
        Recorder::begin(Box::new(sink), PathBuf::from("run.ninput"), &machine).expect("header");
    record_all(&mut recorder, &[Input::new(); 5]);
    // The header, the one failed frame, and no attempt after it.
    assert_eq!(*attempts.lock().expect("not poisoned"), 2);
}

#[test]
fn given_a_sink_that_refuses_the_header_when_recording_begins_then_it_is_an_error() {
    let sink = FailsAfter {
        ok: 0,
        attempts: Arc::default(),
    };
    let result = Recorder::begin(Box::new(sink), PathBuf::from("run.ninput"), &smoke());
    assert_eq!(
        result.err().map(|error| error.to_string()),
        Some("disk full".to_string())
    );
}

#[test]
fn given_a_path_that_does_not_exist_when_creating_the_log_then_the_file_is_made() {
    let path = std::env::temp_dir().join(format!("ninge-record-{}-new.ninput", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let made = create_new(&path).is_ok();
    let exists = path.exists();
    let _ = std::fs::remove_file(&path);
    assert_eq!((made, exists), (true, true));
}

#[test]
fn given_a_path_that_exists_when_creating_the_log_then_it_is_refused_and_left_alone() {
    let path = std::env::temp_dir().join(format!("ninge-record-{}-old.ninput", std::process::id()));
    std::fs::write(&path, "keep me").expect("temp dir is writable");
    let kind = create_new(&path).err().map(|error| error.kind());
    let kept = std::fs::read_to_string(&path).expect("still there");
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        (kind, kept),
        (Some(io::ErrorKind::AlreadyExists), "keep me".to_string())
    );
}
