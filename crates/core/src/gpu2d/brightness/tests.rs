// SPDX-License-Identifier: Apache-2.0
//! The master brightness (issue #34). Every expected value is worked out by hand from
//! `docs/hw/blending.md`, never from the implementation: a 15-bit channel `c` is widened to 6 bits
//! as `c << 1 | c >> 4`, moved by the factor in sixteenths with the quotient truncated, and expanded
//! to 8 bits as `c6 << 2 | c6 >> 4`; with the unit off the channel is `c << 3 | c >> 2`.

use super::*;

const UP: u16 = 1 << 14;
const DOWN: u16 = 2 << 14;
const RESERVED: u16 = 3 << 14;

/// A register value, a 15-bit colour and the xRGB8888 pixel wanted.
struct Case {
    register: u16,
    color: u16,
    want: u32,
}

const fn case(register: u16, color: u16, want: u32) -> Case {
    Case {
        register,
        color,
        want,
    }
}

/// The (index, got) of the cases whose pixel is not the one wanted.
fn expand_misses(cases: &[Case]) -> Vec<(usize, u32)> {
    cases
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let got = MasterBrightness::decode(c.register).expand(c.color);
            (got != c.want).then_some((i, got))
        })
        .collect()
}

#[test]
fn given_brightness_up_when_a_pixel_is_expanded_then_each_6_bit_channel_moves_toward_63() {
    // Black at 8/16: 63 * 8 / 16 = 31 (truncated), expanded 31 << 2 | 31 >> 4 = 125. Red 31 widens to
    // 63, green and blue 0 to 0 and move to 31.
    let cases = [
        case(UP | 8, 0x0000, 0x007D_7D7D),
        case(UP | 16, 0x0000, 0x00FF_FFFF),
        case(UP | 8, 0x001F, 0x00FF_7D7D),
        // 5-bit 16 widens to 33; at 4/16: 33 + 30 * 4 / 16 = 33 + 7 = 40, expanded 162.
        case(UP | 4, 0x4210, 0x00A2_A2A2),
    ];
    assert_eq!(expand_misses(&cases), vec![]);
}

#[test]
fn given_brightness_down_when_a_pixel_is_expanded_then_each_6_bit_channel_moves_toward_0() {
    // White is 63 in every channel; at 8/16 it is 63 - 63 * 8 / 16 = 63 - 31 = 32, expanded 130.
    let cases = [
        case(DOWN | 8, 0x7FFF, 0x0082_8282),
        case(DOWN | 16, 0x7FFF, 0),
        case(DOWN | 8, 0x001F, 0x0082_0000),
        case(DOWN, 0x7FFF, 0x00FF_FFFF),
    ];
    assert_eq!(expand_misses(&cases), vec![]);
}

#[test]
fn given_a_factor_above_16_when_expanded_then_it_counts_as_16() {
    let cases = [
        case(UP | 31, 0x0000, 0x00FF_FFFF),
        case(DOWN | 17, 0x7FFF, 0),
        case(UP | 20, 0x4210, 0x00FF_FFFF),
    ];
    assert_eq!(expand_misses(&cases), vec![]);
}

#[test]
fn given_the_unit_off_or_reserved_when_expanded_then_the_plain_5_to_8_bit_colour_shows() {
    // 5-bit 16 expands to 16 << 3 | 16 >> 2 = 132, whatever the factor.
    let cases = [
        case(8, 0x4210, 0x0084_8484),
        case(RESERVED | 8, 0x4210, 0x0084_8484),
        case(0, 0x4210, 0x0084_8484),
        case(RESERVED | 16, 0x7FFF, 0x00FF_FFFF),
    ];
    assert_eq!(expand_misses(&cases), vec![]);
}

#[test]
fn given_brightness_up_with_factor_0_when_expanded_then_the_6_bit_path_is_taken() {
    // 5-bit 16 widens to 33 and expands to 33 << 2 | 33 >> 4 = 134, not the 132 of the off path.
    let cases = [case(UP, 0x4210, 0x0086_8686), case(UP, 0x7FFF, 0x00FF_FFFF)];
    assert_eq!(expand_misses(&cases), vec![]);
}

#[test]
fn given_bit_15_of_the_colour_set_when_expanded_then_it_has_no_effect() {
    let cases = [case(UP | 8, 0x8000, 0x007D_7D7D)];
    assert_eq!(expand_misses(&cases), vec![]);
}

/// A register value and the (mode, factor) it decodes to.
struct Decoded {
    register: u16,
    want: (BrightnessMode, u8),
}

fn decode_misses(cases: &[Decoded]) -> Vec<usize> {
    cases
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let decoded = MasterBrightness::decode(c.register);
            ((decoded.mode, decoded.factor) != c.want).then_some(i)
        })
        .collect()
}

#[test]
fn given_register_values_when_decoded_then_the_mode_and_the_clamped_factor_follow_the_note() {
    let cases = [
        Decoded {
            register: 0,
            want: (BrightnessMode::Off, 0),
        },
        Decoded {
            register: 0x4010,
            want: (BrightnessMode::Up, 16),
        },
        Decoded {
            register: 0x8003,
            want: (BrightnessMode::Down, 3),
        },
        Decoded {
            register: 0xC01F,
            want: (BrightnessMode::Reserved, 16),
        },
        Decoded {
            register: 0x4011,
            want: (BrightnessMode::Up, 16),
        },
    ];
    assert_eq!(decode_misses(&cases), vec![]);
}
