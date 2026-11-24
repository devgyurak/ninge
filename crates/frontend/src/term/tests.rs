// SPDX-License-Identifier: Apache-2.0
//! Tests of `term`.

use super::*;
use std::io::pipe;

const OK_AND_DA1: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22;52c";

#[test]
fn given_answer_already_written_when_reading_then_it_returns_before_the_deadline() {
    let (reader, mut writer) = pipe().expect("pipe");
    writer.write_all(OK_AND_DA1).expect("write");
    let started = Instant::now();
    let got = read_reply(reader, Duration::from_secs(30));
    assert_eq!(
        (got, started.elapsed() < Duration::from_secs(10)),
        (OK_AND_DA1.to_vec(), true)
    );
}

#[test]
fn given_no_answer_when_reading_then_it_gives_up_after_the_timeout_with_nothing() {
    let (reader, _writer) = pipe().expect("pipe");
    let got = read_reply(reader, Duration::from_millis(50));
    assert_eq!(got, Vec::<u8>::new());
}

#[test]
fn given_partial_answer_and_closed_writer_when_reading_then_it_returns_what_arrived() {
    let (reader, mut writer) = pipe().expect("pipe");
    writer.write_all(b"\x1b_Gi=31;OK").expect("write");
    drop(writer);
    let got = read_reply(reader, Duration::from_secs(30));
    assert_eq!(got, b"\x1b_Gi=31;OK".to_vec());
}

#[test]
fn given_answer_in_two_writes_when_reading_then_it_waits_for_the_da1() {
    let (reader, mut writer) = pipe().expect("pipe");
    writer.write_all(b"\x1b_Gi=31;OK\x1b\\").expect("write");
    let late = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        writer.write_all(b"\x1b[?62c").expect("write");
    });
    let got = read_reply(reader, Duration::from_secs(30));
    late.join().expect("writer thread");
    assert_eq!(got, b"\x1b_Gi=31;OK\x1b\\\x1b[?62c".to_vec());
}

#[test]
fn given_a_successful_run_when_the_outcome_is_made_then_the_note_is_handed_back() {
    let outcome = outcome(Ok(()), Some("save chip eeprom-8k".to_string()));
    assert_eq!(
        outcome.expect("ok"),
        Some("save chip eeprom-8k".to_string())
    );
}

#[test]
fn given_a_failed_run_with_a_note_when_the_outcome_is_made_then_the_error_carries_the_note() {
    let error = outcome(
        Err(anyhow::anyhow!("draw failed")),
        Some("save chip x".to_string()),
    )
    .expect_err("failed");
    assert_eq!(format!("{error:#}"), "save chip x: draw failed");
}

#[test]
fn given_a_failed_run_with_no_note_when_the_outcome_is_made_then_the_error_is_unchanged() {
    let error = outcome(Err(anyhow::anyhow!("draw failed")), None).expect_err("failed");
    assert_eq!(format!("{error:#}"), "draw failed");
}

#[test]
fn given_a_refused_device_when_the_sound_note_is_made_then_it_says_why_there_is_no_sound() {
    assert_eq!(
        sound_note(Some("no sound output device"), false),
        Some("no sound: no sound output device".to_string())
    );
}

#[test]
fn given_a_device_that_failed_when_the_sound_note_is_made_then_it_says_the_run_went_on_silent() {
    assert_eq!(
        sound_note(None, true),
        Some("the sound device reported an error; ran without sound".to_string())
    );
}

#[test]
fn given_a_working_device_or_none_wanted_when_the_sound_note_is_made_then_there_is_none() {
    assert_eq!(sound_note(None, false), None);
}

#[test]
fn given_two_notes_when_joined_then_they_are_on_two_lines_and_one_alone_is_kept() {
    assert_eq!(
        (
            join_notes(Some("a".into()), Some("b".into())),
            join_notes(None, Some("b".into())),
            join_notes(Some("a".into()), None),
            join_notes(None, None)
        ),
        (
            Some("a\nb".to_string()),
            Some("b".to_string()),
            Some("a".to_string()),
            None
        )
    );
}

#[test]
fn given_a_dropped_writer_when_hung_up_is_asked_then_it_is_true_well_before_the_wait() {
    let (reader, writer) = pipe().expect("pipe");
    drop(writer);
    let started = Instant::now();
    let gone = hung_up(&reader, Duration::from_secs(30));
    assert_eq!(
        (gone, started.elapsed() < Duration::from_secs(10)),
        (true, true)
    );
}

#[test]
fn given_an_open_silent_writer_when_hung_up_is_asked_then_it_is_false_after_the_wait() {
    let (reader, _writer) = pipe().expect("pipe");
    assert!(!hung_up(&reader, Duration::from_millis(50)));
}

#[test]
fn given_data_waiting_and_an_open_writer_when_hung_up_is_asked_then_it_is_false_at_once() {
    let (reader, mut writer) = pipe().expect("pipe");
    writer.write_all(b"x").expect("write");
    let started = Instant::now();
    let gone = hung_up(&reader, Duration::from_secs(30));
    assert_eq!(
        (gone, started.elapsed() < Duration::from_secs(10)),
        (false, true)
    );
}

#[test]
fn given_data_waiting_and_a_dropped_writer_when_hung_up_is_asked_then_it_is_true() {
    let (reader, mut writer) = pipe().expect("pipe");
    writer.write_all(b"x").expect("write");
    drop(writer);
    assert!(hung_up(&reader, Duration::from_secs(30)));
}
