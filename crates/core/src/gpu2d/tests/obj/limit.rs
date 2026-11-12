// SPDX-License-Identifier: Apache-2.0
//! The per-line object budget (issue #31). Every expected value comes from `docs/hw/obj.md` ("The
//! line limit"): 1210 cycles a line, 954 with DISPCNT bit 23; a plain object costs its box
//! width, an affine one 10 plus twice it; objects are looked at in entry order and, once one does not fit,
//! it and every later one is dropped. Each scene puts costly objects off the left of the screen
//! (they cost as much as visible ones) and then 8-wide probe objects, and counts the probes that
//! show; the arithmetic is worked out in each test's comment.

use super::affine::put_group;
use super::*;

const HBLANK_FREE: u32 = 1 << 23;
const AFFINE: u16 = 1 << 8;
const DOUBLE: u16 = 1 << 9;
/// X = -256, so a box of up to 128 pixels is entirely off the screen.
const OFFSCREEN: u16 = 0x100;
const LINE: u16 = 40;

/// A 64x32 plain object off the screen at line `LINE`: 64 cycles.
fn big(extra_attr0: u16) -> [u16; 3] {
    [LINE | 1 << 14 | extra_attr0, OFFSCREEN | 3 << 14, 0]
}

/// A 32x32 affine object off the screen at line `LINE`: 74 cycles, or 138 with double-size.
fn affine_off(double: bool) -> [u16; 3] {
    let bits = if double { DOUBLE } else { 0 };
    [LINE | AFFINE | bits, OFFSCREEN | 2 << 14, 0]
}

/// An 8x8 probe of tile 1 (solid 1) at column `8 * i` on line `LINE`: 8 cycles.
fn probe(i: u16) -> [u16; 3] {
    [LINE, 8 * i, 1]
}

/// A machine whose OAM holds `entries` from entry 0, with the identity in affine group 0 and
/// `extra` DISPCNT bits on `side`.
fn crowded(side: Side, extra: u32, entries: &[[u16; 3]]) -> Machine {
    let mut m = scene(side, extra);
    // Tiles 1-4 are solid 1, so an 8x8 probe and an 8x32 one (1D mapping) both show.
    (1..5).for_each(|t| solid4(&mut m, side, t * 32, 1));
    entries
        .iter()
        .enumerate()
        .fold((), |(), (i, &attrs)| put_obj(&mut m, side, i as u32, attrs));
    put_group(&mut m, side, 0, [0x100, 0, 0, 0x100]);
    m
}

/// For each probe `8 * i` (i < `count`) whether its first pixel shows on the side's screen.
fn probes_shown(m: &mut Machine, side: Side, first_probe: u16, count: u16) -> Vec<bool> {
    m.render_frame();
    (0..count)
        .map(|i| {
            get(
                m,
                side,
                usize::from(8 * (first_probe + i)),
                usize::from(LINE),
            ) != BACKDROP
        })
        .collect()
}

/// How many of the probes (entries after the costly ones) show, on `side` with `extra` DISPCNT
/// bits, after `costly` objects.
fn shown_after(side: Side, extra: u32, costly: &[[u16; 3]], probes: u16) -> usize {
    let entries: Vec<_> = costly
        .iter()
        .copied()
        .chain((0..probes).map(probe))
        .collect();
    let mut m = crowded(side, extra, &entries);
    probes_shown(&mut m, side, 0, probes)
        .into_iter()
        .filter(|&shown| shown)
        .count()
}

fn repeat(object: [u16; 3], times: usize) -> Vec<[u16; 3]> {
    vec![object; times]
}

#[test]
fn given_18_costly_objects_when_probes_follow_then_7_fit_in_a_budget_of_1210() {
    // 18 * 64 = 1152 leaves 58 cycles: 7 probes of 8 (56), the 8th does not fit.
    assert_eq!(shown_after(SIDE_A, 0, &repeat(big(0), 18), 20), 7);
}

#[test]
fn given_14_costly_objects_and_bit_23_when_probes_follow_then_7_fit_in_a_budget_of_954() {
    // 14 * 64 = 896 leaves 58 cycles: 7 probes.
    assert_eq!(shown_after(SIDE_A, HBLANK_FREE, &repeat(big(0), 14), 20), 7);
}

#[test]
fn given_14_costly_objects_and_no_bit_23_when_probes_follow_then_all_20_fit_in_a_budget_of_1210() {
    // 896 leaves 314 cycles, room for 39 probes.
    assert_eq!(shown_after(SIDE_A, 0, &repeat(big(0), 14), 20), 20);
}

#[test]
fn given_18_costly_objects_and_bit_23_when_probes_follow_then_the_15th_stops_everything() {
    // 14 * 64 = 896 leaves 58: the 15th object (64) does not fit, so it and every later object,
    // the probes included, are dropped.
    assert_eq!(shown_after(SIDE_A, HBLANK_FREE, &repeat(big(0), 18), 20), 0);
}

#[test]
fn given_engine_b_with_bit_23_when_probes_follow_then_its_own_budget_of_954_applies() {
    assert_eq!(shown_after(SIDE_B, HBLANK_FREE, &repeat(big(0), 14), 20), 7);
}

#[test]
fn given_engine_b_without_bit_23_when_probes_follow_then_its_budget_is_1210() {
    assert_eq!(shown_after(SIDE_B, 0, &repeat(big(0), 18), 20), 7);
}

#[test]
fn given_costs_that_add_up_to_exactly_the_budget_when_a_probe_follows_then_the_last_fits_and_the_next_does_not()
 {
    // 18 * 64 = 1152, an affine 8x8 (10 + 2 * 8 = 26) and a plain 32-wide object (32) make 1210
    // exactly; the affine one and the wide one are the first two probes' places, the third is
    // the 8-cycle probe that no longer fits.
    let mut entries = repeat(big(0), 18);
    entries.push([LINE | AFFINE, 0, 1]);
    entries.push([LINE | 1 << 14, 40 | 1 << 14, 1]);
    entries.push(probe(10));
    let mut m = crowded(SIDE_A, 0, &entries);
    m.render_frame();
    // The affine 8x8 at (0, 40) with the identity shows tile 1; the 32x8 object at column 40 too;
    // the probe at column 80 does not.
    let shown = |x: usize| get(&m, SIDE_A, x, usize::from(LINE)) != BACKDROP;
    assert_eq!([shown(0), shown(40), shown(80)], [true, true, false]);
}

#[test]
fn given_objects_that_do_not_reach_the_line_when_probes_follow_then_they_cost_nothing() {
    // 40 plain 64x32 objects on lines 100-131 do not reach line 40.
    let far = [100 | 1 << 14, OFFSCREEN | 3 << 14, 0];
    assert_eq!(shown_after(SIDE_A, 0, &repeat(far, 40), 20), 20);
}

#[test]
fn given_disabled_objects_when_probes_follow_then_they_cost_nothing_as_the_note_models_unverified()
{
    // TODO(hw-unverified): obj.md "Unverified", a disabled object costs nothing.
    assert_eq!(shown_after(SIDE_A, 0, &repeat(big(1 << 9), 40), 20), 20);
}

#[test]
fn given_16_affine_objects_when_probes_follow_then_each_costs_74_and_3_probes_fit() {
    // 16 * (10 + 2 * 32) = 1184 leaves 26: 3 probes (24).
    assert_eq!(
        shown_after(SIDE_A, 0, &repeat(affine_off(false), 16), 20),
        3
    );
}

#[test]
fn given_8_double_size_affine_objects_when_probes_follow_then_each_costs_138_and_13_probes_fit() {
    // 8 * (10 + 2 * 64) = 1104 leaves 106: 13 probes (104).
    assert_eq!(shown_after(SIDE_A, 0, &repeat(affine_off(true), 8), 20), 13);
}

#[test]
fn given_obj_window_objects_when_probes_follow_then_they_cost_their_cycles_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): obj.md "Unverified", a mode 2 object costs as any other.
    assert_eq!(shown_after(SIDE_A, 0, &repeat(big(2 << 10), 18), 20), 7);
}

#[test]
fn given_an_object_that_does_not_fit_when_a_cheaper_one_follows_then_the_later_one_is_dropped_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): obj.md "Unverified", the objects after the one that does not fit.
    // 18 * 64 = 1152 leaves 58; 6 probes take 48, leaving 10; a visible 64-wide object (64) does
    // not fit; the probe after it (8) would fit in the 10 left but is dropped.
    let mut entries = repeat(big(0), 18);
    entries.extend((0..6).map(probe));
    entries.push([LINE | 1 << 14, 128 | 3 << 14, 1]);
    entries.push(probe(10));
    let mut m = crowded(SIDE_A, 0, &entries);
    m.render_frame();
    let shown = |x: usize| get(&m, SIDE_A, x, usize::from(LINE)) != BACKDROP;
    assert_eq!([shown(40), shown(128), shown(80)], [true, false, false]);
}

#[test]
fn given_a_line_the_costly_objects_do_not_reach_when_drawn_then_the_budget_is_not_used_up_there() {
    // The costly objects are on lines 40-71; the probes are 8x32 on lines 20-51 (Y = 20), so on
    // line 30 all 20 show and on line 45 only the 7 that fit.
    let mut entries = repeat(big(0), 18);
    entries.extend((0..20).map(|i| [20 | 2 << 14, (8 * i) | (1 << 14), 1]));
    let mut m = crowded(SIDE_A, MAPPING_1D, &entries);
    m.render_frame();
    let shown_at = |line: usize| {
        (0..20)
            .filter(|&i| get(&m, SIDE_A, 8 * i, line) != BACKDROP)
            .count()
    };
    assert_eq!((shown_at(30), shown_at(45)), (20, 7));
}
