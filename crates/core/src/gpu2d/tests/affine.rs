// SPDX-License-Identifier: Apache-2.0
//! Affine layers (issue #30). Every expected value comes from `docs/hw/bg-modes.md` ("Affine
//! layers", "Reference point and parameters"), worked out by a small model written here from the
//! note's formulas, never from running the renderer.

use super::*;
use crate::cart::test_image::{Image, words};

/// The registers and memory of one engine, as the tests address them.
#[derive(Clone, Copy)]
struct Eng {
    dispcnt: u32,
    bg_regs: u32,
    affine: u32,
    vram: u32,
    palette: u32,
    screen: Screen,
}

const ENG_A: Eng = Eng {
    dispcnt: DISPCNT_A_ADDR,
    bg_regs: BG_REGS_ADDR,
    affine: AFFINE_REGS_ADDR,
    vram: 0x0600_0000,
    palette: 0x0500_0000,
    screen: Screen::Top,
};
const ENG_B: Eng = Eng {
    dispcnt: DISPCNT_B_ADDR,
    bg_regs: BG_REGS_B_ADDR,
    affine: AFFINE_REGS_B_ADDR,
    vram: 0x0620_0000,
    palette: 0x0500_0400,
    screen: Screen::Bottom,
};

/// VRAMCNT_C, and the two ways the engines' BG VRAM is mapped: bank A MST 1, bank C MST 4.
const VRAMCNT_C: u32 = 0x0400_0242;
const BANK_A_BG: u8 = 0x81;
const BANK_C_BG_B: u8 = 0x84;
/// Character block 4: the tile data sits at 64 KiB, clear of the biggest map (16 KiB).
const CHAR_BLOCK_4: u16 = 4 << 2;
const CHAR_BASE: u32 = 4 * 0x4000;
const WRAP: u16 = 1 << 13;
/// Display mode 1, BG mode 2 (BG2 and BG3 affine), BG2 on.
const MODE2_BG2: u32 = (1 << 16) | 2 | (1 << 10);
/// The backdrop, palette entry 0: blue.
const BACKDROP_BLUE: u32 = 0x0000_00FF;

/// The transform of a scene: PA-PD as written, and X and Y as signed 20.8 numbers.
#[derive(Clone, Copy)]
struct Params {
    pa: i16,
    pb: i16,
    pc: i16,
    pd: i16,
    x: i32,
    y: i32,
}

const IDENTITY: Params = Params {
    pa: 0x100,
    pb: 0,
    pc: 0,
    pd: 0x100,
    x: 0,
    y: 0,
};

/// The 15-bit colour of palette entry `i`, as the scenes set it: distinct for 1..=80.
fn color15(i: u32) -> u16 {
    ((i & 31) | ((3 * i) & 31) << 5 | ((5 * i) & 31) << 10) as u16
}

/// A 15-bit colour as the xRGB the renderer makes (`display-mode-vram.md`, "Colour expansion").
fn expand15(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// The palette value at texel (`tx`, `ty`) of the scene's layer: the tile of cell (cx, cy) is
/// `1 + ((cx + 2 cy) mod 4)`, and its pixel (r, c) holds `16 tile + ((3r + c) mod 16)`.
fn texel_value(tx: i64, ty: i64) -> u32 {
    let tile = 1 + ((tx / 8 + 2 * (ty / 8)) & 3);
    (16 * tile + ((3 * (ty % 8) + tx % 8) & 15)) as u32
}

/// Where the model puts engine screen pixel (`x`, `y`) when the line starts at `origin`, a pair
/// of 20.8 numbers: the texel, floored.
fn texel_at(p: Params, origin: (i32, i32), x: usize, size: i64) -> (i64, i64, bool) {
    let u = i64::from(origin.0) + i64::from(p.pa) * x as i64;
    let v = i64::from(origin.1) + i64::from(p.pc) * x as i64;
    let (tx, ty) = (u.div_euclid(256), v.div_euclid(256));
    (tx, ty, (0..size).contains(&tx) && (0..size).contains(&ty))
}

/// The xRGB pixel the note gives at (`x`, `y`) for a layer of `size` pixels with the line starting
/// at `origin`.
fn expected(p: Params, origin: (i32, i32), x: usize, size: i64, wrap: bool) -> u32 {
    let (tx, ty, inside) = texel_at(p, origin, x, size);
    match (inside, wrap) {
        (false, false) => BACKDROP_BLUE,
        _ => expand15(color15(texel_value(
            tx.rem_euclid(size),
            ty.rem_euclid(size),
        ))),
    }
}

/// Palette, tile data and map of the scene, and the BG VRAM bank of `eng`.
fn scene_memory(m: &mut Machine, eng: Eng, size: u32) {
    let (bank_reg, bank) = match eng.screen {
        Screen::Top => (VRAMCNT_A, BANK_A_BG),
        Screen::Bottom => (VRAMCNT_C, BANK_C_BG_B),
    };
    w8(m, bank_reg, bank);
    w16(m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    w16(m, eng.palette, 0x7C00);
    (1..=80).for_each(|i| w16(m, eng.palette + i * 2, color15(i)));
    (1..=4u32).for_each(|tile| tile_rows(m, eng.vram + CHAR_BASE + tile * 64, tile));
    let cells = size / 8;
    (0..cells * cells / 2).for_each(|pair| {
        let (cx, cy) = (pair * 2 % cells, pair * 2 / cells);
        let tile = |cx: u32| (1 + ((cx + 2 * cy) & 3)) as u16;
        w16(m, eng.vram + pair * 2, tile(cx) | tile(cx + 1) << 8);
    });
}

/// The 64 bytes of tile `tile`: row r, column c holds `16 tile + ((3r + c) mod 16)`.
fn tile_rows(m: &mut Machine, at: u32, tile: u32) {
    (0..8u32).for_each(|r| {
        let byte = |c: u32| (16 * tile + ((3 * r + c) & 15)) as u8;
        w32(
            m,
            at + r * 8,
            u32::from_le_bytes([byte(0), byte(1), byte(2), byte(3)]),
        );
        w32(
            m,
            at + r * 8 + 4,
            u32::from_le_bytes([byte(4), byte(5), byte(6), byte(7)]),
        );
    });
}

/// The transform registers of BG2 (layer 0) or BG3 (layer 1).
fn set_params(m: &mut Machine, eng: Eng, layer: u32, p: Params) {
    let at = eng.affine + layer * 16;
    [p.pa, p.pb, p.pc, p.pd]
        .iter()
        .enumerate()
        .for_each(|(i, &v)| w16(m, at + i as u32 * 2, v as u16));
    w32(m, at + 8, p.x as u32);
    w32(m, at + 12, p.y as u32);
}

/// An engine with the scene's BG2 layer: size code `size_code` (0-3), wrap as given.
fn scene(eng: Eng, size_code: u16, wrap: bool, p: Params) -> Machine {
    let mut m = Machine::new();
    scene_memory(&mut m, eng, 128 << size_code);
    let cnt = CHAR_BLOCK_4 | (size_code << 14) | if wrap { WRAP } else { 0 };
    w16(&mut m, eng.bg_regs + 4, cnt);
    set_params(&mut m, eng, 0, p);
    w32(&mut m, eng.dispcnt, MODE2_BG2);
    m
}

/// The pixels of the drawn frame that differ from the note's picture, as (x, y, got, want).
fn picture_failures(
    m: &mut Machine,
    eng: Eng,
    p: Params,
    size_code: u16,
    wrap: bool,
) -> Vec<(usize, usize, u32, u32)> {
    m.render_frame();
    let size = 128i64 << size_code;
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .filter_map(|(x, y)| {
            // The line starts where the transform has walked to: `y` lines of PB and PD.
            let origin = (
                p.x + i32::from(p.pb) * y as i32,
                p.y + i32::from(p.pd) * y as i32,
            );
            let want = expected(p, origin, x, size, wrap);
            let got = pixel(m, eng.screen, x, y);
            (got != want).then_some((x, y, got, want))
        })
        .take(8)
        .collect()
}

type Diff = Vec<(usize, usize, u32, u32)>;

fn drawn(eng: Eng, size_code: u16, wrap: bool, p: Params) -> Diff {
    picture_failures(&mut scene(eng, size_code, wrap, p), eng, p, size_code, wrap)
}

const NONE: Diff = Vec::new();

const SHEAR: Params = Params {
    pa: 0xE0,
    pb: -0x80,
    pc: 0x80,
    pd: 0xE0,
    x: 0x4000,
    y: -0x2000,
};

// ---------------------------------------------------------------- the picture

#[test]
fn given_an_identity_transform_when_drawn_then_every_pixel_is_the_texel_beneath_it() {
    assert_eq!(drawn(ENG_A, 1, true, IDENTITY), NONE);
}

#[test]
fn given_a_half_step_when_drawn_then_the_layer_is_magnified_two_times() {
    let p = Params {
        pa: 0x80,
        pd: 0x80,
        ..IDENTITY
    };
    assert_eq!(drawn(ENG_A, 1, true, p), NONE);
}

#[test]
fn given_a_transpose_when_drawn_then_screen_x_walks_down_the_layer_and_y_across() {
    let p = Params {
        pa: 0,
        pb: 0x100,
        pc: 0x100,
        pd: 0,
        ..IDENTITY
    };
    assert_eq!(drawn(ENG_A, 1, true, p), NONE);
}

#[test]
fn given_a_rotation_with_a_shear_and_an_origin_when_drawn_then_the_picture_matches_the_model() {
    assert_eq!(drawn(ENG_A, 1, true, SHEAR), NONE);
}

#[test]
fn given_the_same_rotation_without_wrap_when_drawn_then_outside_the_layer_is_transparent() {
    assert_eq!(drawn(ENG_A, 1, false, SHEAR), NONE);
}

#[test]
fn given_a_mirroring_pa_when_drawn_then_the_layer_runs_backward_across_the_line() {
    let p = Params {
        pa: -0x100,
        x: 255 * 256,
        ..IDENTITY
    };
    assert_eq!(drawn(ENG_A, 1, true, p), NONE);
}

#[test]
fn given_an_origin_left_of_the_layer_when_drawn_with_wrap_off_then_the_first_pixels_are_backdrop() {
    let p = Params {
        x: -64 * 256,
        y: -16 * 256,
        ..IDENTITY
    };
    assert_eq!(drawn(ENG_A, 1, false, p), NONE);
}

#[test]
fn given_an_origin_left_of_the_layer_when_drawn_with_wrap_on_then_the_layer_wraps_in() {
    let p = Params {
        x: -64 * 256,
        y: -16 * 256,
        ..IDENTITY
    };
    assert_eq!(drawn(ENG_A, 1, true, p), NONE);
}

#[test]
fn given_a_128_pixel_layer_with_wrap_off_when_drawn_then_only_its_square_shows() {
    assert_eq!(drawn(ENG_A, 0, false, IDENTITY), NONE);
}

#[test]
fn given_a_128_pixel_layer_with_wrap_on_when_drawn_then_it_repeats_across_the_screen() {
    assert_eq!(drawn(ENG_A, 0, true, IDENTITY), NONE);
}

#[test]
fn given_a_512_pixel_layer_when_drawn_with_a_rotation_then_the_picture_matches_the_model() {
    assert_eq!(drawn(ENG_A, 2, true, SHEAR), NONE);
}

#[test]
fn given_a_1024_pixel_layer_when_drawn_with_a_rotation_then_the_picture_matches_the_model() {
    assert_eq!(drawn(ENG_A, 3, false, SHEAR), NONE);
}

#[test]
fn given_a_fraction_in_the_origin_when_drawn_then_the_texel_is_floored() {
    // 255/256 of a texel is still the texel before; a step of 1/256 in PA moves it at pixel 1.
    let p = Params {
        pa: 0x101,
        x: 0x0000_00FF,
        y: 0x0000_01FF,
        ..IDENTITY
    };
    assert_eq!(drawn(ENG_A, 1, true, p), NONE);
}

#[test]
fn given_a_negative_fractional_origin_when_drawn_then_the_texel_rounds_toward_minus_infinity() {
    let p = Params {
        x: -1,
        y: -0x101,
        ..IDENTITY
    };
    assert_eq!(drawn(ENG_A, 1, false, p), NONE);
}

#[test]
fn given_the_layer_on_engine_b_when_drawn_then_the_lower_screen_shows_the_same_model() {
    assert_eq!(drawn(ENG_B, 1, true, SHEAR), NONE);
}

#[test]
fn given_the_layer_on_engine_b_with_wrap_off_when_drawn_then_the_lower_screen_shows_the_model() {
    assert_eq!(drawn(ENG_B, 2, false, SHEAR), NONE);
}

// ------------------------------------------------------------------ layers together

#[test]
fn given_a_text_layer_in_front_of_an_affine_one_when_drawn_then_the_text_shows_where_it_is_opaque()
{
    // BG0 is text (mode 2), priority 0, one solid 4-bit tile at cell (0, 0) in screen block 8.
    let mut m = scene(ENG_A, 1, true, IDENTITY);
    w32(&mut m, ENG_A.dispcnt, MODE2_BG2 | (1 << 8));
    solid_text_tile(&mut m);
    m.render_frame();
    let want = expand15(color15(1));
    assert_eq!(
        (pixel(&m, Screen::Top, 7, 7), pixel(&m, Screen::Top, 8, 0)),
        (want, expected(IDENTITY, (0, 0), 8, 256, true))
    );
}

/// BG0, text: priority 0, character block 6, screen block 8; tile 1 is solid value 1.
fn solid_text_tile(m: &mut Machine) {
    w16(m, BG_REGS_ADDR, (6 << 2) | (8 << 8));
    w16(m, 0x0600_0000 + 8 * 0x800, 1);
    (0..8).for_each(|i| w32(m, 0x0600_0000 + 6 * 0x4000 + 32 + i * 4, 0x1111_1111));
}

#[test]
fn given_two_affine_layers_when_drawn_then_the_lower_number_is_in_front_at_equal_priority() {
    // BG3 is made transparent everywhere (wrap off, origin far away), so BG2 shows.
    let mut m = scene(ENG_A, 1, true, IDENTITY);
    w16(&mut m, ENG_A.bg_regs + 6, CHAR_BLOCK_4 | (1 << 14));
    set_params(
        &mut m,
        ENG_A,
        1,
        Params {
            x: 4096 * 256,
            ..IDENTITY
        },
    );
    w32(&mut m, ENG_A.dispcnt, MODE2_BG2 | (1 << 11));
    m.render_frame();
    assert_eq!(
        pixel(&m, Screen::Top, 9, 9),
        expected(IDENTITY, (0, 9 * 256), 9, 256, true)
    );
}

// ------------------------------------------------------------------- the registers

/// BG2's layer decoded on engine A.
fn bg2(m: &Machine) -> AffineBg {
    m.inspect().gpu2d().affine_bg(EngineId::A, 2)
}

#[test]
fn given_a_word_written_to_bg2x_when_decoded_then_the_value_is_sign_extended_from_28_bits() {
    let mut m = Machine::new();
    w32(&mut m, AFFINE_REGS_ADDR + 8, 0x0800_0000);
    w32(&mut m, AFFINE_REGS_ADDR + 12, 0xFFFF_FF00);
    assert_eq!(bg2(&m).reference, [-0x0800_0000, -256]);
}

#[test]
fn given_bits_28_to_31_written_when_decoded_then_they_are_dropped() {
    let mut m = Machine::new();
    w32(&mut m, AFFINE_REGS_ADDR + 8, 0xF000_0100);
    w32(&mut m, AFFINE_REGS_ADDR + 12, 0x1234_5678);
    assert_eq!(bg2(&m).reference, [0x100, 0x0234_5678]);
}

#[test]
fn given_halfwords_written_to_bg3y_when_decoded_then_the_halves_form_the_value() {
    let mut m = Machine::new();
    w16(&mut m, AFFINE_REGS_ADDR + 16 + 12, 0x5678);
    w16(&mut m, AFFINE_REGS_ADDR + 16 + 14, 0x0123);
    assert_eq!(
        m.inspect().gpu2d().affine_bg(EngineId::A, 3).reference[1],
        0x0123_5678
    );
}

#[test]
fn given_pa_to_pd_written_when_decoded_then_they_are_signed_8_8_numbers() {
    let mut m = Machine::new();
    set_params(
        &mut m,
        ENG_A,
        0,
        Params {
            pa: -256,
            pb: 0x7FFF,
            pc: i16::MIN,
            pd: 1,
            x: 0,
            y: 0,
        },
    );
    assert_eq!(bg2(&m).params, [-256, 0x7FFF, i16::MIN, 1]);
}

#[test]
fn given_a_byte_written_to_pb_when_decoded_then_only_that_byte_changes() {
    let mut m = Machine::new();
    w16(&mut m, AFFINE_REGS_ADDR + 2, 0x1234);
    w8(&mut m, AFFINE_REGS_ADDR + 3, 0xAB);
    assert_eq!(bg2(&m).params[1], 0xAB34_u16 as i16);
}

#[test]
fn given_the_transform_registers_after_writes_when_read_then_they_read_zero_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): bg-modes.md "Unverified", reads of the write-only registers.
    let mut m = Machine::new();
    set_params(&mut m, ENG_A, 0, SHEAR);
    assert_eq!(
        (r32(&m, AFFINE_REGS_ADDR), r32(&m, AFFINE_REGS_ADDR + 8)),
        (0, 0)
    );
}

#[test]
fn given_an_affine_layer_registers_after_power_on_when_decoded_then_they_are_all_zero() {
    let m = Machine::new();
    assert_eq!(
        (bg2(&m).params, bg2(&m).reference, bg2(&m).internal),
        ([0; 4], [0; 2], [0; 2])
    );
}

#[test]
fn given_bgcnt_of_an_affine_layer_when_decoded_then_size_wrap_and_bases_come_from_it() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, (2 << 24) | (3 << 27));
    w16(&mut m, BG_REGS_ADDR + 4, 0xA000 | (5 << 8) | (3 << 2) | 2);
    let layer = bg2(&m);
    assert_eq!(
        (
            layer.priority,
            layer.char_base,
            layer.screen_base,
            layer.size,
            layer.wrap
        ),
        (
            2,
            2 * 0x1_0000 + 3 * 0x4000,
            3 * 0x1_0000 + 5 * 0x800,
            512,
            true
        )
    );
}

#[test]
fn given_engine_b_registers_written_when_decoded_then_engine_a_keeps_its_own() {
    let mut m = Machine::new();
    set_params(&mut m, ENG_B, 0, SHEAR);
    assert_eq!(
        (
            m.inspect().gpu2d().affine_bg(EngineId::B, 2).params[0],
            bg2(&m).params[0]
        ),
        (SHEAR.pa, 0)
    );
}

#[test]
fn given_the_transform_of_a_powered_off_engine_b_when_written_then_it_stays_zero() {
    let mut m = Machine::new();
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP & !(1 << 9));
    set_params(&mut m, ENG_B, 0, SHEAR);
    assert_eq!(m.inspect().gpu2d().affine_bg(EngineId::B, 2).params, [0; 4]);
}

// -------------------------------------------------------- the internal reference point

#[test]
fn given_a_write_to_bg2x_when_decoded_then_the_internal_point_has_it_at_once() {
    let mut m = Machine::new();
    set_params(&mut m, ENG_A, 0, SHEAR);
    assert_eq!(bg2(&m).internal, [SHEAR.x, SHEAR.y]);
}

#[test]
fn given_a_byte_written_to_bg2x_when_decoded_then_the_internal_point_has_the_whole_register() {
    let mut m = Machine::new();
    w32(&mut m, AFFINE_REGS_ADDR + 8, 0x0001_0203);
    w8(&mut m, AFFINE_REGS_ADDR + 9, 0xFF);
    assert_eq!(bg2(&m).internal[0], 0x0001_FF03);
}

#[test]
fn given_lines_drawn_when_decoded_then_the_internal_point_has_moved_by_pb_and_pd_for_each() {
    let mut m = Machine::new();
    set_params(&mut m, ENG_A, 0, SHEAR);
    render_lines(&mut m, 3);
    assert_eq!(
        bg2(&m).internal,
        [
            SHEAR.x + 3 * i32::from(SHEAR.pb),
            SHEAR.y + 3 * i32::from(SHEAR.pd)
        ]
    );
}

#[test]
fn given_a_line_past_the_last_visible_one_when_drawn_then_the_internal_point_does_not_move() {
    let mut m = Machine::new();
    set_params(&mut m, ENG_A, 0, SHEAR);
    m.render_line(192);
    assert_eq!(bg2(&m).internal, [SHEAR.x, SHEAR.y]);
}

#[test]
fn given_a_changed_pb_when_a_line_is_drawn_then_the_step_uses_the_new_value() {
    let mut m = Machine::new();
    set_params(&mut m, ENG_A, 0, IDENTITY);
    m.render_line(0);
    w16(&mut m, AFFINE_REGS_ADDR + 2, 0x40);
    m.render_line(1);
    assert_eq!(bg2(&m).internal[0], 0x40);
}

#[test]
fn given_a_frame_rendered_whole_when_a_second_is_rendered_then_it_starts_from_the_registers() {
    let mut m = scene(ENG_A, 1, true, SHEAR);
    m.render_frame();
    m.render_frame();
    let mut once = scene(ENG_A, 1, true, SHEAR);
    once.render_frame();
    assert_eq!(m.frame(Screen::Top), once.frame(Screen::Top));
}

/// Draws lines 0 to `count - 1`.
fn render_lines(m: &mut Machine, count: usize) {
    (0..count).for_each(|y| m.render_line(y));
}

/// Runs rounds until the frame is at line `line`.
fn run_to_line(m: &mut Machine, line: u16) {
    while m.inspect().vcount() < line {
        crate::scheduler::run_round(m);
    }
}

#[test]
fn given_a_frame_running_when_vblank_starts_then_the_internal_point_is_loaded_from_the_registers() {
    let mut m = Machine::new();
    set_params(&mut m, ENG_A, 0, SHEAR);
    run_to_line(&mut m, 100);
    let mid_frame = bg2(&m).internal;
    run_to_line(&mut m, 192);
    assert_eq!(
        (mid_frame[0] != SHEAR.x, bg2(&m).internal),
        (true, [SHEAR.x, SHEAR.y])
    );
}

#[test]
fn given_a_write_to_bg2x_during_a_frame_when_lines_follow_then_they_start_from_the_new_value() {
    let mut m = Machine::new();
    set_params(&mut m, ENG_A, 0, SHEAR);
    run_to_line(&mut m, 100);
    w32(&mut m, AFFINE_REGS_ADDR + 8, 0x1000);
    // Lines 101-105 are drawn at their starts, five steps of PB from the new value.
    run_to_line(&mut m, 105);
    assert_eq!(bg2(&m).internal[0], 0x1000 + 5 * i32::from(SHEAR.pb));
}

/// A machine booted from a program that spins, with the scene in memory.
fn spinning(p: Params) -> Machine {
    let mut image = Image::valid();
    image.arm9_code = words(&[0xEAFF_FFFE]);
    let mut m = Machine::new();
    m.load_rom(image.build()).expect("synthetic image");
    scene_memory(&mut m, ENG_A, 256);
    w16(&mut m, ENG_A.bg_regs + 4, CHAR_BLOCK_4 | (1 << 14) | WRAP);
    set_params(&mut m, ENG_A, 0, p);
    w32(&mut m, ENG_A.dispcnt, MODE2_BG2);
    m
}

/// The pixels of the presented top screen that differ from the note's picture when BG2X changes
/// to `new_x` after line 100 started: lines 0-100 start from `p.x` and lines 101-191 from
/// `new_x`, each stepping by PB and PD.
fn split_failures(p: Params, new_x: i32) -> Diff {
    let mut m = spinning(p);
    m.run_frame();
    run_to_line(&mut m, 100);
    w32(&mut m, AFFINE_REGS_ADDR + 8, new_x as u32);
    m.run_frame();
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .filter_map(|(x, y)| {
            let origin = match y > 100 {
                true => (
                    new_x + i32::from(p.pb) * (y as i32 - 101),
                    p.y + i32::from(p.pd) * y as i32,
                ),
                false => (
                    p.x + i32::from(p.pb) * y as i32,
                    p.y + i32::from(p.pd) * y as i32,
                ),
            };
            let want = expected(p, origin, x, 256, true);
            let got = pixel(&m, Screen::Top, x, y);
            (got != want).then_some((x, y, got, want))
        })
        .take(8)
        .collect()
}

#[test]
fn given_bg2x_written_after_line_100_started_when_the_frame_ends_then_lines_101_on_use_it() {
    assert_eq!(split_failures(SHEAR, 0x8000), NONE);
}

#[test]
fn given_run_frame_and_render_frame_when_the_same_scene_is_drawn_then_the_screens_are_equal() {
    // The per-line event path and the all-lines path both go through the reference point step.
    let mut running = spinning(SHEAR);
    running.run_frame();
    running.run_frame();
    let mut whole = scene(ENG_A, 1, true, SHEAR);
    whole.render_frame();
    assert_eq!(running.frame(Screen::Top), whole.frame(Screen::Top));
}

#[test]
fn given_an_affine_layer_when_saved_mid_frame_and_loaded_then_the_frame_equals_an_uninterrupted_one()
 {
    let mut m = spinning(SHEAR);
    m.run_frame();
    run_to_line(&mut m, 60);
    let mut loaded = Machine::new();
    loaded.load_rom(m.rom().to_vec()).expect("same image");
    let result = loaded.load_state(&m.save_state());
    m.run_frame();
    loaded.run_frame();
    assert_eq!(
        (result, loaded.frame(Screen::Top) == m.frame(Screen::Top)),
        (Ok(()), true)
    );
}
