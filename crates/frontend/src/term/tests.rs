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
