// SPDX-License-Identifier: Apache-2.0
//! HALTCNT as a device. Expected values come from `docs/hw/interrupts.md` ("HALTCNT",
//! "Halt"), never from running the code.

use super::*;

fn written(offset: u32, width: Width, value: u32) -> Power {
    let mut power = Power::new();
    power.write(offset, width, value, 0);
    power
}

#[test]
fn given_a_byte_write_of_mode_2_to_haltcnt_when_written_then_a_halt_is_requested() {
    assert!(written(1, Width::Byte, 0x80).halt_requested());
}

#[test]
fn given_a_write_of_mode_2_with_the_unused_bits_set_when_written_then_a_halt_is_requested() {
    assert!(written(1, Width::Byte, 0xBF).halt_requested());
}

#[test]
fn given_the_other_modes_when_written_then_no_halt_is_requested() {
    let requested: Vec<bool> = [0x00, 0x40, 0xC0, 0x3F]
        .iter()
        .map(|&value| written(1, Width::Byte, value).halt_requested())
        .collect();
    assert_eq!(requested, [false, false, false, false]);
}

#[test]
fn given_a_halfword_write_at_postflg_when_its_high_byte_is_mode_2_then_haltcnt_gets_it() {
    assert!(written(0, Width::Half, 0x8000).halt_requested());
}

#[test]
fn given_a_write_of_mode_2_to_postflg_or_the_unused_bytes_when_written_then_nothing_happens() {
    let requested: Vec<bool> = [0, 2, 3]
        .iter()
        .map(|&offset| written(offset, Width::Byte, 0x80).halt_requested())
        .collect();
    assert_eq!(requested, [false, false, false]);
}

#[test]
fn given_a_pending_request_when_taken_then_it_is_given_once() {
    let mut power = written(1, Width::Byte, 0x80);
    assert_eq!((power.take_halt(), power.take_halt()), (true, false));
}

#[test]
fn given_haltcnt_when_read_then_it_reads_zero() {
    let power = written(1, Width::Byte, 0x80);
    assert_eq!(power.peek(1, Width::Byte, 0), 0);
}
