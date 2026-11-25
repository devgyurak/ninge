// SPDX-License-Identifier: Apache-2.0
//! The input log in memory and as text. Expected texts come from `docs/contributing/input-log.md`.

use super::*;
use ninge_core::cart::SaveType;
use ninge_core::{Buttons, Touch};

fn header() -> LogHeader {
    LogHeader {
        rom_crc: 0x1a2b,
        save_type: SaveType::None,
        save_hash: 0x0011_2233_aabb_ccdd,
    }
}

fn held() -> Input {
    Input::new().with_buttons(Buttons::A | Buttons::START)
}

#[test]
fn given_an_input_log_when_asked_inside_and_past_its_end_then_the_entry_or_the_default_comes_back()
{
    let log = InputLog::new(vec![Input::default(), held()]);
    assert_eq!(
        (log.input_for(0), log.input_for(1), log.input_for(2)),
        (Input::default(), held(), Input::default())
    );
}

#[test]
fn given_a_log_when_written_as_text_then_it_is_the_header_and_one_line_per_frame() {
    let log = InputLog::new(vec![held(), Input::default()]);
    assert_eq!(
        log.to_text(&header()),
        "ninge-input 1 rom=1a2b save=none:00112233aabbccdd\nf=1 b=A,START\nf=2\n"
    );
}

#[test]
fn given_a_log_when_written_and_read_back_then_the_header_and_frames_are_the_same() {
    let touched = Input::new().with_touch(Some(Touch { x: 128, y: 96 }));
    let log = InputLog::new(vec![held(), touched, Input::default()]);
    let read = InputLog::from_text(&log.to_text(&header())).expect("a log it wrote");
    assert_eq!(
        read,
        ReadLog {
            header: header(),
            log,
            dropped_partial_line: false
        }
    );
}

#[test]
fn given_a_text_cut_off_mid_line_when_read_then_the_partial_line_is_dropped_and_said() {
    let text = "ninge-input 1 rom=1a2b save=none:00112233aabbccdd\nf=1 b=A\nf=2 b=";
    let read = InputLog::from_text(text).expect("a prefix is a log");
    assert_eq!((read.log.len(), read.dropped_partial_line), (1, true));
}

#[test]
fn given_a_text_without_a_header_when_read_then_it_is_refused() {
    assert!(InputLog::from_text("f=1 b=A\n").is_err());
}

#[test]
fn given_an_empty_log_when_asked_then_it_is_empty() {
    assert_eq!(
        (InputLog::default().len(), InputLog::default().is_empty()),
        (0, true)
    );
}
