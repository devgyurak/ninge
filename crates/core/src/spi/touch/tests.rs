// SPDX-License-Identifier: Apache-2.0
//! The touch-screen controller, from `docs/hw/touch-power.md` and GBATEK's "DS Touch Screen
//! Controller (TSC)". Expected words are packed by hand from the note's reply layout; a table goes
//! through a harness that returns the rows that differ.

use proptest::prelude::*;

use super::*;
use crate::spi::CALIBRATION;

const START_BYTE: u8 = 0x80;
const X_12: u8 = START_BYTE | (5 << 4);
const Y_12: u8 = START_BYTE | (1 << 4);
const Z1_12: u8 = START_BYTE | (3 << 4) | 4;
const Z2_12: u8 = START_BYTE | (4 << 4) | 4;
const X_8: u8 = X_12 | 0x08;

const fn at(x: u8, y: u8) -> Option<Touch> {
    Some(Touch { x, y })
}

/// A command and its two reply bytes, with the session held until the last byte: the word a
/// program rebuilds as `((hi << 8) | lo) >> 3`, masked to 12 bits.
fn read12(tsc: &mut Tsc, command: u8, panel: Option<Touch>) -> u16 {
    tsc.transfer(command, true, panel);
    let hi = tsc.transfer(0, true, panel);
    let lo = tsc.transfer(0, false, panel);
    (((u16::from(hi) << 8) | u16::from(lo)) >> 3) & 0xFFF
}

/// The reply bytes `(hi, lo)` of one command.
fn bytes(tsc: &mut Tsc, command: u8, panel: Option<Touch>) -> (u8, u8) {
    tsc.transfer(command, true, panel);
    (tsc.transfer(0, true, panel), tsc.transfer(0, false, panel))
}

/// The rows `(name, got, want)` that differ.
fn differing(rows: &[(&'static str, u16, u16)]) -> Vec<(&'static str, u16, u16)> {
    rows.iter().filter(|r| r.1 != r.2).copied().collect()
}

/// GBATEK's conversion for one axis, in integers.
fn to_pixel(adc: u16, adc1: u16, adc2: u16, scr1: u8, scr2: u8) -> i32 {
    (i32::from(adc) - i32::from(adc1)) * (i32::from(scr2) - i32::from(scr1))
        / (i32::from(adc2) - i32::from(adc1))
        + (i32::from(scr1) - 1)
}

/// Every pixel of the panel read back through the chip and GBATEK's formula: the pixels that do
/// not come back.
fn pixels_not_recovered() -> Vec<(u8, u8)> {
    let [(ax1, ay1), (ax2, ay2)] = CALIBRATION.adc;
    let [(sx1, sy1), (sx2, sy2)] = CALIBRATION.screen;
    let mut tsc = Tsc::new();
    (0..=255u8)
        .flat_map(|x| (0..192u8).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            let rx = read12(&mut tsc, X_12, at(x, y));
            let ry = read12(&mut tsc, Y_12, at(x, y));
            to_pixel(rx, ax1, ax2, sx1, sx2) != i32::from(x)
                || to_pixel(ry, ay1, ay2, sy1, sy2) != i32::from(y)
        })
        .collect()
}

#[test]
fn given_a_touch_when_x_and_y_are_read_in_12_bit_mode_then_they_are_16_times_the_pixel() {
    let mut tsc = Tsc::new();
    let rows = [
        ("x origin", read12(&mut tsc, X_12, at(0, 0)), 0),
        ("y origin", read12(&mut tsc, Y_12, at(0, 0)), 0),
        ("x right", read12(&mut tsc, X_12, at(255, 191)), 255 * 16),
        ("y bottom", read12(&mut tsc, Y_12, at(255, 191)), 191 * 16),
        ("x point 1", read12(&mut tsc, X_12, at(32, 32)), 512),
        ("y point 1", read12(&mut tsc, Y_12, at(32, 32)), 512),
        ("x point 2", read12(&mut tsc, X_12, at(223, 159)), 3568),
        ("y point 2", read12(&mut tsc, Y_12, at(223, 159)), 2544),
    ];
    assert_eq!(differing(&rows), vec![]);
}

#[test]
fn given_every_pixel_when_read_through_the_chip_and_the_formula_then_it_comes_back_exact() {
    assert_eq!(pixels_not_recovered(), Vec::<(u8, u8)>::new());
}

proptest! {
    #[test]
    fn given_any_pixel_when_x_is_read_then_it_is_16_times_the_column(x in 0u8..=255, y in 0u8..192) {
        let mut tsc = Tsc::new();
        prop_assert_eq!(read12(&mut tsc, X_12, at(x, y)), u16::from(x) * 16);
    }
}

#[test]
fn given_no_touch_when_x_and_y_are_read_then_x_is_zero_and_y_is_full_scale() {
    let mut tsc = Tsc::new();
    let rows = [
        ("x released", read12(&mut tsc, X_12, None), 0x000),
        ("y released", read12(&mut tsc, Y_12, None), 0xFFF),
    ];
    assert_eq!(differing(&rows), vec![]);
}

#[test]
fn given_a_12_bit_command_when_the_reply_is_read_then_the_bytes_are_packed_after_a_dummy_bit() {
    // Pixel 35 reads 0x230 = 0010_0011_0000; after the dummy zero bit that is 0x230 << 3 = 0x1180.
    let mut tsc = Tsc::new();
    assert_eq!(bytes(&mut tsc, X_12, at(35, 0)), (0b0001_0001, 0b1000_0000));
}

#[test]
fn given_an_8_bit_command_when_the_reply_is_read_then_the_result_is_the_top_8_bits_after_a_dummy_bit()
 {
    // Pixel 35 reads 0x230; the 8-bit result 0x23 follows the dummy bit: 0x23 << 7 = 0x1180.
    let mut tsc = Tsc::new();
    assert_eq!(bytes(&mut tsc, X_8, at(35, 0)), (0b0001_0001, 0b1000_0000));
}

#[test]
fn given_an_8_bit_command_when_rebuilt_with_the_twelve_bit_formula_then_it_is_the_result_shifted_left_by_4()
 {
    let mut tsc = Tsc::new();
    assert_eq!(read12(&mut tsc, X_8, at(200, 0)), 200 << 4);
}

#[test]
fn given_a_12_bit_command_when_the_command_byte_itself_is_exchanged_then_it_receives_zero() {
    let mut tsc = Tsc::new();
    assert_eq!(tsc.transfer(X_12, true, at(10, 10)), 0);
}

#[test]
fn given_the_next_command_in_the_low_byte_slot_when_read_then_both_conversions_arrive() {
    // A: command X, B: zero (X high), C: command Y (X low), D: zero (Y high), E: zero (Y low).
    let mut tsc = Tsc::new();
    let panel = at(100, 50);
    tsc.transfer(X_12, true, panel);
    let x_hi = tsc.transfer(0, true, panel);
    let x_lo = tsc.transfer(Y_12, true, panel);
    let y_hi = tsc.transfer(0, true, panel);
    let y_lo = tsc.transfer(0, false, panel);
    let rebuilt = |hi: u8, lo: u8| (((u16::from(hi) << 8) | u16::from(lo)) >> 3) & 0xFFF;
    assert_eq!(
        (rebuilt(x_hi, x_lo), rebuilt(y_hi, y_lo)),
        (100 * 16, 50 * 16)
    );
}

#[test]
fn given_a_command_in_the_high_byte_slot_when_sent_then_it_replaces_the_old_reply() {
    // TODO(hw-unverified) in the model: the new command wins and the old tail is dropped.
    let mut tsc = Tsc::new();
    let panel = at(100, 50);
    tsc.transfer(X_12, true, panel);
    tsc.transfer(Y_12, true, panel);
    let hi = tsc.transfer(0, true, panel);
    let lo = tsc.transfer(0, false, panel);
    assert_eq!(
        (((u16::from(hi) << 8) | u16::from(lo)) >> 3) & 0xFFF,
        50 * 16
    );
}

#[test]
fn given_the_session_is_released_mid_reply_when_a_new_session_reads_then_the_old_reply_is_gone() {
    let mut tsc = Tsc::new();
    // Pixel 35 has a non-zero low reply byte (0x80), so a reply that survived would show.
    tsc.transfer(X_12, true, at(35, 50));
    tsc.transfer(0, false, at(35, 50));
    assert_eq!(tsc.transfer(0, true, at(35, 50)), 0);
}

#[test]
fn given_a_byte_without_the_start_bit_when_sent_then_the_mode_and_reply_do_not_change() {
    let mut tsc = Tsc::new();
    tsc.transfer(0x7F, true, at(100, 50));
    assert_eq!(
        (tsc.mode(), tsc.command(), tsc.transfer(0, true, None)),
        (0, 0, 0)
    );
}

#[test]
fn given_a_reply_read_to_its_end_when_more_bytes_are_clocked_then_they_are_zero() {
    let mut tsc = Tsc::new();
    read12(&mut tsc, X_12, at(100, 50));
    assert_eq!(bytes(&mut tsc, 0, None), (0, 0));
}

/// The pen output for each mode 0-3, after a command in that mode.
fn pen_enabled_per_mode() -> Vec<bool> {
    (0..4u8)
        .map(|mode| {
            let mut tsc = Tsc::new();
            tsc.transfer(X_12 | mode, true, None);
            tsc.pen_irq_enabled()
        })
        .collect()
}

#[test]
fn given_each_power_down_mode_when_a_command_sets_it_then_the_pen_output_is_enabled_in_0_and_2() {
    assert_eq!(pen_enabled_per_mode(), vec![true, false, true, false]);
}

#[test]
fn given_a_power_on_chip_when_the_pen_output_is_asked_then_it_is_enabled() {
    assert!(Tsc::new().pen_irq_enabled());
}

#[test]
fn given_a_reply_cut_by_deselect_when_the_mode_is_asked_then_the_mode_stays() {
    let mut tsc = Tsc::new();
    tsc.transfer(X_12 | 1, false, None);
    assert_eq!((tsc.mode(), tsc.command()), (1, X_12 | 1));
}

#[test]
fn given_the_pressure_channels_when_touched_and_released_then_the_values_are_the_fixed_ones() {
    let mut tsc = Tsc::new();
    let rows = [
        ("z1 touched", read12(&mut tsc, Z1_12, at(1, 1)), 0x600),
        ("z2 touched", read12(&mut tsc, Z2_12, at(1, 1)), 0xA00),
        ("z1 released", read12(&mut tsc, Z1_12, None), 0x000),
        ("z2 released", read12(&mut tsc, Z2_12, None), 0xFFF),
    ];
    assert_eq!(differing(&rows), vec![]);
}

#[test]
fn given_the_single_ended_channels_when_read_then_each_gives_its_fixed_value() {
    let single = |channel: u8| START_BYTE | (channel << 4) | 4;
    let mut tsc = Tsc::new();
    let rows = [
        ("temp0", read12(&mut tsc, single(0), None), 0x600),
        ("battery", read12(&mut tsc, single(2), None), 0x000),
        ("aux", read12(&mut tsc, single(6), None), 0x800),
        ("temp1", read12(&mut tsc, single(7), None), 0x68E),
    ];
    assert_eq!(differing(&rows), vec![]);
}

#[test]
fn given_the_two_temperature_channels_when_put_in_gbateks_formula_then_it_is_about_297_kelvin() {
    let single = |channel: u8| START_BYTE | (channel << 4) | 4;
    let mut tsc = Tsc::new();
    let tp0 = u32::from(read12(&mut tsc, single(0), None));
    let tp1 = u32::from(read12(&mut tsc, single(7), None));
    assert_eq!((tp1 - tp0) * 8568 / 4096, 297);
}

#[test]
fn given_a_differential_command_on_a_single_ended_channel_when_read_then_it_is_zero() {
    let differential = |channel: u8| START_BYTE | (channel << 4);
    let mut tsc = Tsc::new();
    let rows = [
        ("temp0", read12(&mut tsc, differential(0), None), 0),
        ("battery", read12(&mut tsc, differential(2), None), 0),
        ("aux", read12(&mut tsc, differential(6), None), 0),
        ("temp1", read12(&mut tsc, differential(7), None), 0),
    ];
    assert_eq!(differing(&rows), vec![]);
}

#[test]
fn given_a_single_ended_position_command_when_read_then_it_equals_the_differential_one() {
    let mut tsc = Tsc::new();
    let panel = at(77, 33);
    assert_eq!(
        (
            read12(&mut tsc, X_12 | 4, panel),
            read12(&mut tsc, Y_12 | 4, panel)
        ),
        (77 * 16, 33 * 16)
    );
}

#[test]
fn given_a_fresh_chip_when_checked_then_it_is_well_formed() {
    assert!(Tsc::new().well_formed());
}

#[test]
fn given_a_chip_mid_reply_when_checked_then_it_is_well_formed() {
    let mut tsc = Tsc::new();
    tsc.transfer(X_12, true, at(255, 191));
    tsc.transfer(0, true, at(255, 191));
    assert!(tsc.well_formed());
}

/// States that no sequence of transfers reaches, built field by field.
fn malformed() -> Vec<Tsc> {
    let fresh = Tsc::new();
    vec![
        Tsc { mode: 4, ..fresh },
        Tsc { left: 3, ..fresh },
        Tsc { reply: 1, ..fresh },
        Tsc {
            left: 2,
            reply: 0x8000,
            ..fresh
        },
        Tsc {
            left: 2,
            reply: 0x0001,
            ..fresh
        },
        Tsc {
            left: 1,
            reply: 0x0100,
            ..fresh
        },
    ]
}

#[test]
fn given_states_the_chip_cannot_reach_when_checked_then_none_is_well_formed() {
    let accepted: Vec<Tsc> = malformed().into_iter().filter(Tsc::well_formed).collect();
    assert_eq!(accepted, vec![]);
}
