// SPDX-License-Identifier: Apache-2.0
//! The power-management device, from `docs/hw/touch-power.md` and GBATEK's "DS Power Management
//! Device". A table goes through a harness that returns the rows that differ.

use super::*;

const WRITE: u8 = 0x00;
const READ_BIT: u8 = 0x80;

/// A write session: the index byte held, then the data byte releasing the chip.
fn write_reg(pm: &mut PowerManagement, index: u8, value: u8) {
    pm.transfer(WRITE | index, true);
    pm.transfer(value, false);
}

/// A read session: the index byte held, then a dummy byte whose reply is the register.
fn read_reg(pm: &mut PowerManagement, index: u8) -> u8 {
    pm.transfer(READ_BIT | index, true);
    pm.transfer(0, false)
}

/// The rows `(name, got, want)` that differ.
fn differing(rows: &[(&'static str, u8, u8)]) -> Vec<(&'static str, u8, u8)> {
    rows.iter().filter(|r| r.1 != r.2).copied().collect()
}

#[test]
fn given_the_control_register_when_written_with_all_ones_then_bits_0_to_6_read_back() {
    let mut pm = PowerManagement::new();
    write_reg(&mut pm, 0, 0xFF);
    assert_eq!(read_reg(&mut pm, 0), 0x7F);
}

/// The rows `(name, got, want)` for each named control bit written alone.
fn named_bits_read_back() -> Vec<(&'static str, u8, u8)> {
    let mut pm = PowerManagement::new();
    [
        ("amp", CONTROL_AMP),
        ("lower backlight", CONTROL_LOWER_BACKLIGHT),
        ("upper backlight", CONTROL_UPPER_BACKLIGHT),
        ("shutdown", CONTROL_SHUTDOWN),
    ]
    .into_iter()
    .map(|(name, bit)| {
        write_reg(&mut pm, 0, bit);
        (name, read_reg(&mut pm, 0), bit)
    })
    .collect()
}

#[test]
fn given_the_control_register_when_each_named_bit_is_written_then_it_reads_back_alone() {
    assert_eq!(differing(&named_bits_read_back()), vec![]);
}

#[test]
fn given_the_microphone_amplifier_register_when_written_with_all_ones_then_only_bit_0_reads_back() {
    let mut pm = PowerManagement::new();
    write_reg(&mut pm, 2, 0xFF);
    assert_eq!(read_reg(&mut pm, 2), 0x01);
}

#[test]
fn given_the_gain_register_when_written_with_all_ones_then_only_bits_0_and_1_read_back() {
    let mut pm = PowerManagement::new();
    write_reg(&mut pm, 3, 0xFF);
    assert_eq!(read_reg(&mut pm, 3), 0x03);
}

#[test]
fn given_the_battery_register_when_written_then_it_is_dropped_and_reads_power_good() {
    let mut pm = PowerManagement::new();
    write_reg(&mut pm, 1, 0xFF);
    assert_eq!(read_reg(&mut pm, 1), 0x00);
}

#[test]
fn given_a_fresh_device_when_every_register_is_read_then_register_0_has_the_amplifier_and_backlights_on()
 {
    let mut pm = PowerManagement::new();
    let rows = [
        ("control", read_reg(&mut pm, 0), 0x0D),
        ("battery", read_reg(&mut pm, 1), 0),
        ("amp", read_reg(&mut pm, 2), 0),
        ("gain", read_reg(&mut pm, 3), 0),
    ];
    assert_eq!(differing(&rows), vec![]);
}

#[test]
fn given_a_held_write_session_when_more_bytes_follow_then_each_writes_the_same_register() {
    // TODO(hw-unverified) in the model: a further byte of a session repeats the access.
    let mut pm = PowerManagement::new();
    pm.transfer(WRITE, true);
    pm.transfer(0x04, true);
    pm.transfer(0x08, false);
    assert_eq!(read_reg(&mut pm, 0), 0x08);
}

#[test]
fn given_an_index_above_3_when_read_then_it_mirrors_registers_0_to_3() {
    let mut pm = PowerManagement::new();
    write_reg(&mut pm, 0, 0x0C);
    write_reg(&mut pm, 2, 1);
    write_reg(&mut pm, 3, 2);
    let rows = [
        ("4 is 0", read_reg(&mut pm, 4), 0x0C),
        ("6 is 2", read_reg(&mut pm, 6), 1),
        ("7 is 3", read_reg(&mut pm, 7), 2),
        ("7Fh is 3", read_reg(&mut pm, 0x7F), 2),
    ];
    assert_eq!(differing(&rows), vec![]);
}

#[test]
fn given_a_write_to_a_mirror_when_register_0_is_read_then_it_holds_the_value() {
    // The DS-Lite's register 4 does not exist here: on the original console it is register 0.
    let mut pm = PowerManagement::new();
    write_reg(&mut pm, 4, 0x44);
    assert_eq!(read_reg(&mut pm, 0), 0x44);
}

#[test]
fn given_the_index_byte_when_exchanged_then_the_chip_answers_zero() {
    let mut pm = PowerManagement::new();
    write_reg(&mut pm, 0, 0x0C);
    assert_eq!(pm.transfer(READ_BIT, true), 0);
}

#[test]
fn given_a_write_when_the_data_byte_is_exchanged_then_the_chip_answers_zero() {
    let mut pm = PowerManagement::new();
    pm.transfer(WRITE, true);
    assert_eq!(pm.transfer(0x0C, false), 0);
}

#[test]
fn given_a_released_session_when_the_next_byte_arrives_then_it_is_an_index_again() {
    // After the data byte releases the chip, the next byte is a read index, not more data.
    let mut pm = PowerManagement::new();
    write_reg(&mut pm, 0, 0x08);
    pm.transfer(READ_BIT, false);
    assert_eq!(read_reg(&mut pm, 0), 0x08);
}

#[test]
fn given_extra_bytes_in_a_read_session_when_exchanged_then_the_register_repeats() {
    let mut pm = PowerManagement::new();
    write_reg(&mut pm, 0, 0x30);
    pm.transfer(READ_BIT, true);
    let first = pm.transfer(0, true);
    let second = pm.transfer(0, false);
    assert_eq!((first, second), (0x30, 0x30));
}

#[test]
fn given_a_reachable_state_when_checked_then_it_is_well_formed() {
    let mut pm = PowerManagement::new();
    write_reg(&mut pm, 0, 0xFF);
    write_reg(&mut pm, 2, 0xFF);
    write_reg(&mut pm, 3, 0xFF);
    assert!(pm.well_formed());
}

/// States no write reaches, built field by field.
fn malformed() -> Vec<PowerManagement> {
    let fresh = PowerManagement::new();
    vec![
        PowerManagement {
            control: 0x80,
            ..fresh
        },
        PowerManagement { amp: 2, ..fresh },
        PowerManagement { gain: 4, ..fresh },
    ]
}

#[test]
fn given_states_the_device_cannot_reach_when_checked_then_none_is_well_formed() {
    let accepted: Vec<PowerManagement> = malformed()
        .into_iter()
        .filter(PowerManagement::well_formed)
        .collect();
    assert_eq!(accepted, vec![]);
}
