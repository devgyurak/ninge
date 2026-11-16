// SPDX-License-Identifier: Apache-2.0
//! Colour special effects through the whole machine (issue #34). Every expected value is worked
//! out by hand from `docs/hw/blending.md`, never from the implementation. The scene is the one of
//! `window.rs`: four solid text layers, BG0 red in front to BG3 yellow behind, over a white
//! backdrop; red is 31,0,0, green 0,31,0, blue 0,0,31, yellow 31,31,0 and magenta 31,0,31.

use super::affine::{ENG_A, ENG_B, Eng};
use super::window::{
    BG0, BG1, EFFECT, GRAPHICS_BGS, MAGENTA, OBJ, OBJ_ON, RED, WHITE15, WIN0_ON, WIN0H, WIN0V,
    WININ, WINOUT, YELLOW, layers_scene, misses, prepare_objects, put_object, range,
};
use super::*;
use crate::debug::{EffectMode, EngineId};

/// Offsets of the effect registers from an engine's DISPCNT.
const BLDCNT: u32 = 0x50;
const BLDALPHA: u32 = 0x52;
const BLDY: u32 = 0x54;

/// BLDCNT fields: the layer bits as first targets, the second targets above them, the effect.
const fn second(layers: u16) -> u16 {
    layers << 8
}
const ALPHA: u16 = 1 << 6;
const BRIGHTEN: u16 = 2 << 6;
const DARKEN: u16 = 3 << 6;
const BD: u16 = 1 << 5;
const BG2: u16 = 4;
const BG3: u16 = 8;
/// BLDALPHA from EVA and EVB.
const fn alpha(eva: u16, evb: u16) -> u16 {
    eva | evb << 8
}

/// A screen position, the registers and the colour wanted there.
struct Case {
    bld: [u16; 3],
    at: (usize, usize),
    want: u16,
}

const fn at(bld: [u16; 3], x: usize, y: usize, want: u16) -> Case {
    Case {
        bld,
        at: (x, y),
        want,
    }
}

/// The (x, y, got, want) of the cases whose pixel is not the one wanted. Each case runs on a machine
/// of its own: `dispcnt` and `regs` set up the scene, `objects` fill OAM from entry 0, and the
/// effect registers come from the case.
fn blend_misses(
    eng: Eng,
    dispcnt: u32,
    regs: &[(u32, u16)],
    objects: &[[u16; 3]],
    cases: &[Case],
) -> Vec<(usize, usize, u32, u32)> {
    cases
        .iter()
        .filter_map(|case| {
            let mut m = layers_scene(eng, dispcnt, regs);
            prepare_objects(&mut m);
            objects
                .iter()
                .enumerate()
                .for_each(|(entry, &attrs)| put_object(&mut m, entry as u32, attrs));
            [BLDCNT, BLDALPHA, BLDY]
                .iter()
                .zip(case.bld)
                .for_each(|(&offset, value)| w16(&mut m, eng.dispcnt + offset, value));
            let (x, y) = case.at;
            misses(&mut m, eng, &[(x, y, case.want)]).first().copied()
        })
        .collect()
}

const MIXED: u16 = 15 | 15 << 5;
/// Magenta over red at 8/8: red 31, blue 15.
const MAGENTA_ON_RED: u16 = 31 | 15 << 10;

// ------------------------------------------------------------------------- backgrounds

#[test]
fn given_bg0_first_and_bg1_second_when_alpha_coefficients_vary_then_the_pixel_is_the_mix() {
    let bld = |eva, evb| [1 | ALPHA | second(BG1), alpha(eva, evb), 0];
    let cases = [
        at(bld(8, 8), 5, 7, MIXED),
        at(bld(4, 12), 5, 7, 7 | 23 << 5),
        at(bld(20, 3), 5, 7, 31 | 5 << 5),
        at(bld(31, 31), 5, 7, RED | 0x03E0),
        at(bld(16, 0), 200, 150, RED),
        at(bld(0, 16), 200, 150, 0x03E0),
    ];
    assert_eq!(blend_misses(ENG_A, GRAPHICS_BGS, &[], &[], &cases), vec![]);
}

#[test]
fn given_a_first_target_when_brightness_changes_then_the_pixel_moves_toward_white_or_black() {
    let up = |evy| [1 | BRIGHTEN, 0, evy];
    let down = |evy| [1 | DARKEN, 0, evy];
    let cases = [
        at(up(8), 5, 7, 31 | 15 << 5 | 15 << 10),
        at(up(16), 5, 7, WHITE15),
        at(up(31), 5, 7, WHITE15),
        at(up(0), 5, 7, RED),
        at(down(8), 5, 7, 16),
        at(down(16), 5, 7, 0),
        at(down(31), 5, 7, 0),
    ];
    assert_eq!(blend_misses(ENG_A, GRAPHICS_BGS, &[], &[], &cases), vec![]);
}

#[test]
fn given_targets_that_do_not_line_up_when_drawn_then_the_front_layer_shows_unchanged() {
    let eva = alpha(8, 8);
    let cases = [
        // BG1 sits between BG0 and the second target BG2.
        at([1 | ALPHA | second(BG2), eva, 0], 5, 7, RED),
        // No second target.
        at([1 | ALPHA, eva, 0], 5, 7, RED),
        // BG0 is the second target and BG1 the first: the second is above the first.
        at([2 | ALPHA | second(BG0), eva, 0], 5, 7, RED),
        // The effect is none.
        at([1 | second(BG1), eva, 0], 5, 7, RED),
        // BG1 is the first target of a brightness effect but BG0 is in front.
        at([2 | BRIGHTEN, 0, 16], 5, 7, RED),
    ];
    assert_eq!(blend_misses(ENG_A, GRAPHICS_BGS, &[], &[], &cases), vec![]);
}

#[test]
fn given_all_layers_selected_as_both_targets_when_drawn_then_the_top_and_the_next_mix() {
    let cases = [at(
        [0x3F | ALPHA | second(0x3F), alpha(8, 8), 0],
        5,
        7,
        MIXED,
    )];
    assert_eq!(blend_misses(ENG_A, GRAPHICS_BGS, &[], &[], &cases), vec![]);
}

#[test]
fn given_the_backdrop_as_a_target_when_drawn_then_it_blends_and_darkens_like_a_layer() {
    // Only BG3 is on: yellow 31,31,0 over the white backdrop. 15 is 31 * 8 / 16 truncated.
    let only_bg3 = 1 << 16 | 1 << 11;
    let over_backdrop = [BG3 | ALPHA | second(BD), alpha(8, 8), 0];
    let bg3_cases = [
        at(over_backdrop, 5, 7, 31 | 31 << 5 | 15 << 10),
        at([BG3 | ALPHA, alpha(8, 8), 0], 5, 7, YELLOW),
    ];
    // No layer at all: the backdrop is on top, darkened at 8/16 to 16 in every channel.
    let none = 1 << 16;
    let backdrop_cases = [
        at([BD | DARKEN, 0, 8], 5, 7, 16 | 16 << 5 | 16 << 10),
        at([BD | ALPHA | second(BD), alpha(8, 8), 0], 5, 7, WHITE15),
    ];
    assert_eq!(
        (
            blend_misses(ENG_A, only_bg3, &[], &[], &bg3_cases),
            blend_misses(ENG_A, none, &[], &[], &backdrop_cases)
        ),
        (vec![], vec![])
    );
}

// --------------------------------------------------------------------------- windows

/// Window 0 covers columns 0-127 and every row; `inside` and `outside` are its groups.
fn window_regs(inside: u16, outside: u16) -> [(u32, u16); 4] {
    [
        (WIN0H, range(0, 128)),
        (WIN0V, range(0, 192)),
        (WININ, inside),
        (WINOUT, outside),
    ]
}

#[test]
fn given_the_effect_bit_set_only_inside_a_window_when_drawn_then_only_the_inside_blends() {
    let bld = [1 | ALPHA | second(BG1), alpha(8, 8), 0];
    let regs = window_regs(BG0 | BG1 | EFFECT, BG0 | BG1);
    let cases = [at(bld, 10, 7, MIXED), at(bld, 200, 7, RED)];
    assert_eq!(
        blend_misses(ENG_A, GRAPHICS_BGS | WIN0_ON, &regs, &[], &cases),
        vec![]
    );
}

#[test]
fn given_the_effect_bit_set_only_outside_a_window_when_drawn_then_only_the_outside_blends() {
    let bld = [1 | ALPHA | second(BG1), alpha(8, 8), 0];
    let regs = window_regs(BG0 | BG1, BG0 | BG1 | EFFECT);
    let cases = [at(bld, 10, 7, RED), at(bld, 200, 7, MIXED)];
    assert_eq!(
        blend_misses(ENG_A, GRAPHICS_BGS | WIN0_ON, &regs, &[], &cases),
        vec![]
    );
}

#[test]
fn given_the_effect_bit_clear_when_a_brightness_effect_is_on_then_the_region_is_not_changed() {
    let bld = [1 | DARKEN, 0, 16];
    let regs = window_regs(BG0 | BG1, BG0 | BG1 | EFFECT);
    let cases = [at(bld, 10, 7, RED), at(bld, 200, 7, 0)];
    assert_eq!(
        blend_misses(ENG_A, GRAPHICS_BGS | WIN0_ON, &regs, &[], &cases),
        vec![]
    );
}

#[test]
fn given_no_window_on_when_winout_has_no_effect_bit_then_the_effect_works_everywhere() {
    let bld = [1 | ALPHA | second(BG1), alpha(8, 8), 0];
    let regs = [(WINOUT, BG0 | BG1)];
    let cases = [at(bld, 10, 7, MIXED), at(bld, 200, 100, MIXED)];
    assert_eq!(
        blend_misses(ENG_A, GRAPHICS_BGS, &regs, &[], &cases),
        vec![]
    );
}

// ---------------------------------------------------------------------------- objects

/// An 8 by 8 object at (10, 10), colour magenta (`window.rs`).
fn sprite(attr0_bits: u16, priority: u16) -> [u16; 3] {
    [10 | attr0_bits, 10, priority << 10]
}

/// Attr0 mode 1: semi-transparent.
const SEMI: u16 = 1 << 10;

#[test]
fn given_a_normal_object_when_it_is_or_is_not_a_first_target_then_only_a_first_target_blends() {
    let eva = alpha(8, 8);
    let cases = [
        at([OBJ | ALPHA | second(BG0), eva, 0], 12, 12, MAGENTA_ON_RED),
        at([ALPHA | second(BG0), eva, 0], 12, 12, MAGENTA),
    ];
    assert_eq!(
        blend_misses(ENG_A, GRAPHICS_BGS | OBJ_ON, &[], &[sprite(0, 0)], &cases),
        vec![]
    );
}

#[test]
fn given_a_semi_transparent_object_when_bldcnt_selects_nothing_then_it_blends_with_a_second_target()
{
    let eva = alpha(8, 8);
    let cases = [
        // No effect and bit 4 clear, yet the object blends with BG0 below it.
        at([second(BG0), eva, 0], 12, 12, MAGENTA_ON_RED),
        // BG1 is the second target but the pixel below is BG0: no blend.
        at([second(BG1), eva, 0], 12, 12, MAGENTA),
        // The pixel next to the object is BG0 alone: unchanged.
        at([second(BG0), eva, 0], 30, 12, RED),
    ];
    assert_eq!(
        blend_misses(
            ENG_A,
            GRAPHICS_BGS | OBJ_ON,
            &[],
            &[sprite(SEMI, 0)],
            &cases
        ),
        vec![]
    );
}

#[test]
fn given_a_semi_transparent_object_when_a_brightness_effect_is_on_then_blending_has_priority() {
    let eva = alpha(8, 8);
    let cases = [
        // Over a second target: blended, not brightened.
        at([BRIGHTEN | second(BG0), eva, 16], 12, 12, MAGENTA_ON_RED),
        at([DARKEN | second(BG0), eva, 16], 12, 12, MAGENTA_ON_RED),
        // Over a non-target: the brightness effect takes the object as a first target.
        at([BRIGHTEN | second(BG1), 0, 16], 12, 12, WHITE15),
        at([DARKEN | second(BG1), 0, 16], 12, 12, 0),
        // The background beside it is not a first target.
        at([BRIGHTEN | second(BG1), 0, 16], 30, 12, RED),
    ];
    assert_eq!(
        blend_misses(
            ENG_A,
            GRAPHICS_BGS | OBJ_ON,
            &[],
            &[sprite(SEMI, 0)],
            &cases
        ),
        vec![]
    );
}

#[test]
fn given_a_semi_transparent_object_when_the_window_effect_bit_differs_then_the_bit_gates_the_blend()
{
    // Window 0 is columns 0-127, so the object at x = 10 is inside it.
    let bld = [second(BG0), alpha(8, 8), 0];
    let clear = window_regs(BG0 | OBJ, BG0 | OBJ);
    let set = window_regs(BG0 | OBJ | EFFECT, BG0 | OBJ);
    let dispcnt = GRAPHICS_BGS | OBJ_ON | WIN0_ON;
    assert_eq!(
        (
            blend_misses(
                ENG_A,
                dispcnt,
                &clear,
                &[sprite(SEMI, 0)],
                &[at(bld, 12, 12, MAGENTA)]
            ),
            blend_misses(
                ENG_A,
                dispcnt,
                &set,
                &[sprite(SEMI, 0)],
                &[at(bld, 12, 12, MAGENTA_ON_RED)]
            )
        ),
        (vec![], vec![])
    );
}

#[test]
fn given_an_object_behind_bg0_when_it_is_the_second_target_then_bg0_blends_with_it() {
    // Only BG0 and the objects are on, so the object (priority 1) is the pixel below BG0 (0).
    let dispcnt = 1 << 16 | 1 << 8 | OBJ_ON;
    let eva = alpha(8, 8);
    let cases = [
        at([1 | ALPHA | second(OBJ), eva, 0], 12, 12, MAGENTA_ON_RED),
        // The object is not a second target: BG0 shows alone.
        at([1 | ALPHA | second(BD), eva, 0], 12, 12, RED),
    ];
    assert_eq!(
        blend_misses(ENG_A, dispcnt, &[], &[sprite(0, 1)], &cases),
        vec![]
    );
}

#[test]
fn given_an_object_between_bg0_and_bg1_when_bg0_blends_then_the_object_is_the_pixel_below() {
    // Only BG0 and BG1 are on; BG1 has priority 1, like the object, which is in front of it: the
    // order is BG0, object, BG1.
    let dispcnt = 1 << 16 | 3 << 8 | OBJ_ON;
    let bg1cnt = (ENG_A.bg_regs - ENG_A.dispcnt + 2, 1 << 2 | 1 << 8 | 1);
    let regs = [(bg1cnt.0, bg1cnt.1)];
    let eva = alpha(8, 8);
    let cases = [
        at([1 | ALPHA | second(OBJ), eva, 0], 12, 12, MAGENTA_ON_RED),
        at([1 | ALPHA | second(BG1), eva, 0], 12, 12, RED),
    ];
    assert_eq!(
        blend_misses(ENG_A, dispcnt, &regs, &[sprite(0, 1)], &cases),
        vec![]
    );
}

// ----------------------------------------------------------------------- engine B

#[test]
fn given_engine_b_with_its_own_registers_when_drawn_then_the_lower_screen_blends() {
    let bld = [1 | ALPHA | second(BG1), alpha(8, 8), 0];
    let cases = [at(bld, 5, 7, MIXED)];
    assert_eq!(blend_misses(ENG_B, GRAPHICS_BGS, &[], &[], &cases), vec![]);
}

#[test]
fn given_engine_a_blending_when_engine_b_has_no_effect_then_the_lower_screen_is_not_blended() {
    let mut m = layers_scene(ENG_B, GRAPHICS_BGS, &[]);
    w16(&mut m, ENG_A.dispcnt + BLDCNT, 1 | ALPHA | second(BG1));
    w16(&mut m, ENG_A.dispcnt + BLDALPHA, alpha(8, 8));
    assert_eq!(misses(&mut m, ENG_B, &[(5, 7, RED)]), vec![]);
}

// --------------------------------------------------------------------------- registers

/// A machine where every bit of the effect registers of both engines has been written.
fn all_bits_written() -> Machine {
    let mut m = Machine::new();
    [DISPCNT_A_ADDR, DISPCNT_B_ADDR].iter().for_each(|&base| {
        [BLDCNT, BLDALPHA, BLDY]
            .iter()
            .for_each(|&offset| w16(&mut m, base + offset, 0xFFFF));
    });
    m
}

#[test]
fn given_all_bits_written_when_the_effect_registers_are_read_then_the_unused_bits_read_0() {
    let m = all_bits_written();
    let read = |base: u32| [BLDCNT, BLDALPHA, BLDY].map(|offset| r16(&m, base + offset));
    // BLDY is write-only: it reads 0 though EVY was stored (see the decoded view below).
    assert_eq!(
        (read(DISPCNT_A_ADDR), read(DISPCNT_B_ADDR)),
        ([0x3FFF, 0x1F1F, 0], [0x3FFF, 0x1F1F, 0])
    );
}

#[test]
fn given_a_word_write_at_bldcnt_when_read_then_it_sets_bldcnt_and_bldalpha() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR + BLDCNT, 0x0804_0241);
    assert_eq!(
        (
            r16(&m, DISPCNT_A_ADDR + BLDCNT),
            r16(&m, DISPCNT_A_ADDR + BLDALPHA)
        ),
        (0x0241, 0x0804)
    );
}

#[test]
fn given_byte_writes_when_bldcnt_is_read_then_each_byte_changes_only_itself() {
    let mut m = Machine::new();
    w8(&mut m, DISPCNT_A_ADDR + BLDCNT, 0x41);
    w8(&mut m, DISPCNT_A_ADDR + BLDCNT + 1, 0xFF);
    assert_eq!(r16(&m, DISPCNT_A_ADDR + BLDCNT), 0x3F41);
}

#[test]
fn given_a_write_to_the_halfword_after_bldy_when_read_then_it_is_no_register() {
    let mut m = Machine::new();
    w16(&mut m, DISPCNT_A_ADDR + BLDY + 2, 0xFFFF);
    assert_eq!(r16(&m, DISPCNT_A_ADDR + BLDY + 2), 0);
}

#[test]
fn given_registers_written_when_inspected_then_the_decoded_view_is_the_note_s() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR + BLDCNT, 20 << 16 | 0x0241);
    w16(&mut m, DISPCNT_A_ADDR + BLDY, 0x1F);
    let e = m.inspect().gpu2d().effects(EngineId::A);
    assert_eq!(
        (
            e.mode,
            e.first.bg(0),
            e.first.bg(1),
            e.second.bg(1),
            e.second.backdrop(),
            e.eva,
            e.evb,
            e.evy
        ),
        (EffectMode::Alpha, true, false, true, false, 16, 0, 16)
    );
}

#[test]
fn given_engine_b_powered_off_when_bldcnt_is_written_then_the_write_is_ignored() {
    let mut m = Machine::new();
    w16(&mut m, POWCNT1_ADDR, 0x820F & !(1 << 9));
    w16(&mut m, DISPCNT_B_ADDR + BLDCNT, 0x0241);
    assert_eq!(r16(&m, DISPCNT_B_ADDR + BLDCNT), 0);
}
