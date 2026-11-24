// SPDX-License-Identifier: Apache-2.0
//! HALTCNT as a device. Expected values come from `docs/hw/interrupts.md` ("HALTCNT",
//! "Halt"), never from running the code.

use super::*;
use crate::io::device::cx_at;

/// Whether a write of `value` asks for a halt.
fn halts(offset: u32, width: Width, value: u32) -> bool {
    Power::new().write(offset, width, value, cx_at(0)).halt
}

fn written(offset: u32, width: Width, value: u32) -> Power {
    let mut power = Power::new();
    power.write(offset, width, value, cx_at(0));
    power
}

#[test]
fn given_a_byte_write_of_mode_2_to_haltcnt_when_written_then_the_effect_asks_for_a_halt() {
    assert!(halts(1, Width::Byte, 0x80));
}

#[test]
fn given_a_write_of_mode_2_with_the_unused_bits_set_when_written_then_the_effect_asks_for_a_halt() {
    assert!(halts(1, Width::Byte, 0xBF));
}

#[test]
fn given_the_other_modes_when_written_then_no_halt_is_requested() {
    let requested: Vec<bool> = [0x00, 0x40, 0xC0, 0x3F]
        .iter()
        .map(|&value| halts(1, Width::Byte, value))
        .collect();
    assert_eq!(requested, [false, false, false, false]);
}

#[test]
fn given_a_halfword_write_at_postflg_when_its_high_byte_is_mode_2_then_haltcnt_gets_it() {
    assert!(halts(0, Width::Half, 0x8000));
}

#[test]
fn given_a_write_of_mode_2_to_postflg_or_the_unused_bytes_when_written_then_nothing_happens() {
    let requested: Vec<bool> = [0, 2, 3]
        .iter()
        .map(|&offset| halts(offset, Width::Byte, 0x80))
        .collect();
    assert_eq!(requested, [false, false, false]);
}

#[test]
fn given_haltcnt_when_read_then_it_reads_zero() {
    let power = written(1, Width::Byte, 0x80);
    assert_eq!(power.peek(1, Width::Byte, cx_at(0)), 0);
}

#[test]
fn given_powcnt2_when_bit_0_is_written_then_the_speakers_follow_it() {
    // Bit 0 is the internal speakers, on at power-on (`spu.md`, "The gates").
    let speakers = [
        Power::new().speakers_on(),
        written(4, Width::Half, 0).speakers_on(),
        written(4, Width::Half, 2).speakers_on(),
        written(4, Width::Half, 3).speakers_on(),
    ];
    assert_eq!(speakers, [true, false, false, true]);
}
