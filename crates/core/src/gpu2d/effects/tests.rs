// SPDX-License-Identifier: Apache-2.0
//! The colour special effects (issue #34). Every expected value is worked out by hand from
//! `docs/hw/blending.md` (GBATEK "LCD I/O Color Special Effects"), never from the implementation.
//! A colour is 15 bits: red in bits 0-4, green in 5-9, blue in 10-14.

use super::*;

const RED: u16 = 0x001F;
const GREEN: u16 = 0x03E0;
const BLUE: u16 = 0x7C00;
const WHITE: u16 = 0x7FFF;
const BLACK: u16 = 0;

const BG0: u8 = 0;
const BG1: u8 = 1;
const BG2: u8 = 2;

const fn pixel(color: u16, layer: u8) -> Pixel {
    Pixel {
        color,
        layer,
        semi: false,
    }
}

const fn semi(color: u16) -> Pixel {
    Pixel {
        color,
        layer: LAYER_OBJ,
        semi: true,
    }
}

/// One call of `apply`: the registers, the top pixel, what is below, and the region's effect bit.
struct Case {
    bldcnt: u16,
    bldalpha: u16,
    bldy: u16,
    top: Pixel,
    below: Option<Pixel>,
    allowed: bool,
    want: u16,
}

const fn case(bldcnt: u16, bldalpha: u16, bldy: u16, top: Pixel, below: Pixel, want: u16) -> Case {
    Case {
        bldcnt,
        bldalpha,
        bldy,
        top,
        below: Some(below),
        allowed: true,
        want,
    }
}

/// The cases whose result is not the one wanted, as (index, got).
fn apply_misses(cases: &[Case]) -> Vec<(usize, u16)> {
    cases
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let effects = Effects::decode(c.bldcnt, c.bldalpha, c.bldy);
            let got = effects.apply(c.top, || c.below, c.allowed);
            (got != c.want).then_some((i, got))
        })
        .collect()
}

/// BLDCNT: BG0 first, BG1 second, alpha blending.
const ALPHA_BG0_BG1: u16 = 1 | 1 << 6 | 1 << 9;

#[test]
fn given_alpha_coefficients_when_two_targets_mix_then_each_channel_is_the_truncated_sum_in_sixteenths()
 {
    let (top, below) = (pixel(RED, BG0), pixel(GREEN, BG1));
    // Red 31,0,0 over green 0,31,0: red = 31 * EVA / 16, green = 31 * EVB / 16, each truncated.
    let cases = [
        case(ALPHA_BG0_BG1, 8 | 8 << 8, 0, top, below, 15 | 15 << 5),
        case(ALPHA_BG0_BG1, 4 | 12 << 8, 0, top, below, 7 | 23 << 5),
        case(ALPHA_BG0_BG1, 1 | 1 << 8, 0, top, below, 1 | 1 << 5),
        case(ALPHA_BG0_BG1, 16 | 16 << 8, 0, top, below, RED | GREEN),
        case(ALPHA_BG0_BG1, 0, 0, top, below, BLACK),
        case(ALPHA_BG0_BG1, 16, 0, top, below, RED),
        case(ALPHA_BG0_BG1, 16 << 8, 0, top, below, GREEN),
    ];
    assert_eq!(apply_misses(&cases), vec![]);
}

#[test]
fn given_coefficients_above_16_when_two_targets_mix_then_they_count_as_16() {
    let (top, below) = (pixel(RED, BG0), pixel(GREEN, BG1));
    let cases = [
        case(ALPHA_BG0_BG1, 31 | 31 << 8, 0, top, below, RED | GREEN),
        case(ALPHA_BG0_BG1, 20 | 3 << 8, 0, top, below, 31 | 5 << 5),
        case(ALPHA_BG0_BG1, 17 | 17 << 8, 0, top, below, RED | GREEN),
    ];
    assert_eq!(apply_misses(&cases), vec![]);
}

#[test]
fn given_a_sum_above_31_when_two_targets_mix_then_the_channel_is_capped_at_31() {
    // White over white at 16/16 each sums to 62 in every channel.
    let cases = [case(
        ALPHA_BG0_BG1,
        16 | 16 << 8,
        0,
        pixel(WHITE, BG0),
        pixel(WHITE, BG1),
        WHITE,
    )];
    assert_eq!(apply_misses(&cases), vec![]);
}

#[test]
fn given_brightness_effects_when_applied_then_the_first_target_moves_toward_white_or_black() {
    let red = pixel(RED, BG0);
    let (up, down) = (1 | 2 << 6, 1 | 3 << 6);
    // Increase: I + (31 - I) * EVY / 16; decrease: I - I * EVY / 16; both truncated.
    let cases = [
        case(up, 0, 0, red, red, RED),
        case(up, 0, 8, red, red, 31 | 15 << 5 | 15 << 10),
        case(up, 0, 1, red, red, 31 | 1 << 5 | 1 << 10),
        case(up, 0, 16, red, red, WHITE),
        case(up, 0, 31, red, red, WHITE),
        case(down, 0, 0, red, red, RED),
        case(down, 0, 8, red, red, 16),
        case(down, 0, 1, red, red, 30),
        case(down, 0, 16, red, red, BLACK),
        case(down, 0, 31, red, red, BLACK),
    ];
    assert_eq!(apply_misses(&cases), vec![]);
}

#[test]
fn given_a_brightness_effect_when_second_targets_and_alpha_are_set_then_they_play_no_part() {
    let red = pixel(RED, BG0);
    let up_with_second = 1 | 2 << 6 | 1 << 9;
    let cases = [case(
        up_with_second,
        8 | 8 << 8,
        8,
        red,
        pixel(GREEN, BG1),
        31 | 15 << 5 | 15 << 10,
    )];
    assert_eq!(apply_misses(&cases), vec![]);
}

#[test]
fn given_targets_that_do_not_line_up_when_applied_then_the_top_pixel_shows_unchanged() {
    let (top, below) = (pixel(RED, BG0), pixel(GREEN, BG1));
    let alpha = 8 | 8 << 8;
    let cases = [
        // The second target is BG2, not the pixel below.
        case(1 | 1 << 6 | 1 << 10, alpha, 0, top, below, RED),
        // No second target at all.
        case(1 | 1 << 6, alpha, 0, top, below, RED),
        // The top pixel is not a first target (BG1 is, and BG0 is the second).
        case(2 | 1 << 6 | 1 << 8, alpha, 0, top, below, RED),
        // The effect is none.
        case(1 | 1 << 9, alpha, 0, top, below, RED),
        // A brightness effect on a layer that is not a first target.
        case(2 | 2 << 6, 0, 16, top, below, RED),
    ];
    assert_eq!(apply_misses(&cases), vec![]);
}

#[test]
fn given_every_layer_selected_as_both_targets_when_applied_then_the_top_is_first_and_the_next_second()
 {
    let cases = [case(
        0x3F | 0x3F << 8 | 1 << 6,
        8 | 8 << 8,
        0,
        pixel(RED, BG0),
        pixel(GREEN, BG1),
        15 | 15 << 5,
    )];
    assert_eq!(apply_misses(&cases), vec![]);
}

#[test]
fn given_the_unused_bldcnt_bits_set_when_applied_then_the_effect_is_the_same() {
    let cases = [case(
        ALPHA_BG0_BG1 | 0xC000,
        8 | 8 << 8,
        0,
        pixel(RED, BG0),
        pixel(GREEN, BG1),
        15 | 15 << 5,
    )];
    assert_eq!(apply_misses(&cases), vec![]);
}

#[test]
fn given_the_backdrop_as_a_target_when_applied_then_it_blends_and_darkens_like_a_layer() {
    let backdrop = |color| pixel(color, LAYER_BACKDROP);
    let cases = [
        // BG2 first and the backdrop second: blue 0,0,31 over white.
        case(
            1 << 2 | 1 << 6 | 1 << 13,
            8 | 8 << 8,
            0,
            pixel(BLUE, BG2),
            backdrop(WHITE),
            15 | 15 << 5 | 31 << 10,
        ),
        // The backdrop as the first target, darkened.
        case(
            1 << 5 | 3 << 6,
            0,
            8,
            backdrop(WHITE),
            backdrop(WHITE),
            16 | 16 << 5 | 16 << 10,
        ),
    ];
    assert_eq!(apply_misses(&cases), vec![]);
}

#[test]
fn given_the_effect_bit_of_the_region_clear_when_applied_then_the_top_pixel_shows_unchanged() {
    let blend = Case {
        bldcnt: ALPHA_BG0_BG1,
        bldalpha: 8 | 8 << 8,
        bldy: 0,
        top: pixel(RED, BG0),
        below: Some(pixel(GREEN, BG1)),
        allowed: false,
        want: RED,
    };
    let darken = Case {
        bldcnt: 1 | 3 << 6,
        bldy: 16,
        ..blend
    };
    assert_eq!(apply_misses(&[blend, darken]), vec![]);
}

#[test]
fn given_no_effect_when_applied_then_the_pixel_below_is_not_asked_for() {
    let effects = Effects::decode(0, 0, 0);
    assert_eq!(
        effects.apply(pixel(RED, BG0), || panic!("asked"), true),
        RED
    );
}

#[test]
fn given_a_brightness_effect_when_applied_then_the_pixel_below_is_not_asked_for() {
    let effects = Effects::decode(1 | 2 << 6, 0, 16);
    assert_eq!(
        effects.apply(pixel(RED, BG0), || panic!("asked"), true),
        WHITE
    );
}

/// BLDCNT with the second target BG0 only: the targets a semi-transparent object tests are
/// the second one, whatever bit 4 and the effect say.
const SECOND_BG0: u16 = 1 << 8;

#[test]
fn given_a_semi_transparent_object_over_a_second_target_when_applied_then_it_blends_whatever_bldcnt_says()
 {
    let magenta = 0x7C1F;
    let (obj, below) = (semi(magenta), pixel(RED, BG0));
    let mixed = 31 | 15 << 10;
    let cases = [
        // No effect, bit 4 clear.
        case(SECOND_BG0, 8 | 8 << 8, 0, obj, below, mixed),
        // A brightness effect: blending has priority and brightness is not applied.
        case(SECOND_BG0 | 2 << 6, 8 | 8 << 8, 16, obj, below, mixed),
        case(SECOND_BG0 | 3 << 6, 8 | 8 << 8, 16, obj, below, mixed),
    ];
    assert_eq!(apply_misses(&cases), vec![]);
}

#[test]
fn given_a_semi_transparent_object_over_a_non_target_when_applied_then_only_a_brightness_effect_changes_it()
 {
    let magenta = 0x7C1F;
    let (obj, below) = (semi(magenta), pixel(GREEN, BG1));
    let cases = [
        case(SECOND_BG0, 8 | 8 << 8, 0, obj, below, magenta),
        case(SECOND_BG0 | 1 << 6, 8 | 8 << 8, 0, obj, below, magenta),
        case(SECOND_BG0 | 2 << 6, 0, 16, obj, below, WHITE),
        case(SECOND_BG0 | 3 << 6, 0, 16, obj, below, BLACK),
    ];
    assert_eq!(apply_misses(&cases), vec![]);
}

#[test]
fn given_a_semi_transparent_object_when_the_region_effect_bit_is_clear_then_it_shows_unchanged() {
    let magenta = 0x7C1F;
    let hidden = Case {
        bldcnt: SECOND_BG0,
        bldalpha: 8 | 8 << 8,
        bldy: 0,
        top: semi(magenta),
        below: Some(pixel(RED, BG0)),
        allowed: false,
        want: magenta,
    };
    assert_eq!(apply_misses(&[hidden]), vec![]);
}

#[test]
fn given_a_semi_transparent_object_over_nothing_when_applied_then_it_is_unchanged() {
    let magenta = 0x7C1F;
    let effects = Effects::decode(SECOND_BG0, 8 | 8 << 8, 0);
    assert_eq!(effects.apply(semi(magenta), || None, true), magenta);
}

#[test]
fn given_a_normal_object_not_selected_as_first_target_when_applied_then_it_is_not_blended() {
    let magenta = 0x7C1F;
    let obj = pixel(magenta, LAYER_OBJ);
    let cases = [
        case(
            1 << 6 | 1 << 8,
            8 | 8 << 8,
            0,
            obj,
            pixel(RED, BG0),
            magenta,
        ),
        // Selected as first target (bit 4), it blends.
        case(
            1 << 4 | 1 << 6 | 1 << 8,
            8 | 8 << 8,
            0,
            obj,
            pixel(RED, BG0),
            31 | 15 << 10,
        ),
    ];
    assert_eq!(apply_misses(&cases), vec![]);
}

/// One BLDCNT, BLDALPHA, BLDY and what decoding them gives.
struct Decoded {
    bldcnt: u16,
    bldalpha: u16,
    bldy: u16,
    want: (EffectMode, [bool; 6], [bool; 6], u8, u8, u8),
}

fn target_bits(set: Targets) -> [bool; 6] {
    [
        set.bg(0),
        set.bg(1),
        set.bg(2),
        set.bg(3),
        set.obj(),
        set.backdrop(),
    ]
}

fn decode_misses(cases: &[Decoded]) -> Vec<usize> {
    cases
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let e = Effects::decode(c.bldcnt, c.bldalpha, c.bldy);
            let got = (
                e.mode,
                target_bits(e.first),
                target_bits(e.second),
                e.eva,
                e.evb,
                e.evy,
            );
            (got != c.want).then_some(i)
        })
        .collect()
}

#[test]
fn given_register_values_when_decoded_then_modes_targets_and_clamped_coefficients_follow_the_note()
{
    let cases = [
        Decoded {
            bldcnt: 0,
            bldalpha: 0,
            bldy: 0,
            want: (EffectMode::None, [false; 6], [false; 6], 0, 0, 0),
        },
        Decoded {
            bldcnt: 0b01_000101 | 0b00_100010 << 8,
            bldalpha: 20 | 5 << 8,
            bldy: 17,
            want: (
                EffectMode::Alpha,
                [true, false, true, false, false, false],
                [false, true, false, false, false, true],
                16,
                5,
                16,
            ),
        },
        Decoded {
            bldcnt: 2 << 6 | 1 << 4,
            bldalpha: 0,
            bldy: 3,
            want: (
                EffectMode::Brighten,
                [false, false, false, false, true, false],
                [false; 6],
                0,
                0,
                3,
            ),
        },
        Decoded {
            bldcnt: 3 << 6 | 1 << 5,
            bldalpha: 0,
            bldy: 0x1F,
            want: (
                EffectMode::Darken,
                [false, false, false, false, false, true],
                [false; 6],
                0,
                0,
                16,
            ),
        },
    ];
    assert_eq!(decode_misses(&cases), vec![]);
}
