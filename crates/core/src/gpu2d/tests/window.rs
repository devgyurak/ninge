// SPDX-License-Identifier: Apache-2.0
//! Windows (issue #33). Every expected value comes from `docs/hw/windows.md`, never from the
//! implementation. The scene is four solid text layers, BG0 red in front to BG3 yellow behind,
//! over a white backdrop, so a pixel's colour names the front-most layer its region shows.

use super::affine::{BANK_A_BG, BANK_C_BG_B, ENG_A, ENG_B, Eng, VRAMCNT_C, expand15};
use super::*;
use crate::gpu2d::window::LayerSet;

/// The colours of BG0-BG3 and the backdrop, as 15-bit values.
const LAYER_COLORS: [u16; 4] = [0x001F, 0x03E0, 0x7C00, 0x03FF];
const RED: u16 = LAYER_COLORS[0];
const GREEN: u16 = LAYER_COLORS[1];
const BLUE: u16 = LAYER_COLORS[2];
const YELLOW: u16 = LAYER_COLORS[3];
const WHITE15: u16 = 0x7FFF;
/// The colour of the test sprite.
const MAGENTA: u16 = 0x7C1F;

/// Offsets of the window registers from an engine's DISPCNT.
const WIN0H: u32 = 0x40;
const WIN1H: u32 = 0x42;
const WIN0V: u32 = 0x44;
const WIN1V: u32 = 0x46;
const WININ: u32 = 0x48;
const WINOUT: u32 = 0x4A;

/// DISPCNT: display mode 1 with BG0-BG3 on, and the bits that turn windows on.
const GRAPHICS_BGS: u32 = 1 << 16 | 0xF << 8;
const WIN0_ON: u32 = 1 << 13;
const WIN1_ON: u32 = 1 << 14;
const OBJ_WIN_ON: u32 = 1 << 15;
const OBJ_ON: u32 = 1 << 12;

/// WININ / WINOUT group bits: BG0-BG3, OBJ and the effect.
const BG0: u16 = 1;
const BG1: u16 = 2;
const BG2: u16 = 4;
const BG3: u16 = 8;
const OBJ: u16 = 1 << 4;
const EFFECT: u16 = 1 << 5;

/// X1 in the high byte and X2 (or Y1 and Y2) in the low byte.
const fn range(start: u16, end: u16) -> u16 {
    start << 8 | end
}

/// A group in the high byte of WININ or WINOUT.
const fn high(group: u16) -> u16 {
    group << 8
}

/// A machine with the four layers of engine `eng` set up and `dispcnt` and the window registers
/// written.
fn layers_scene(eng: Eng, dispcnt: u32, regs: &[(u32, u16)]) -> Machine {
    let mut m = Machine::new();
    let (bank_reg, bank) = match eng.screen {
        Screen::Top => (VRAMCNT_A, BANK_A_BG),
        Screen::Bottom => (VRAMCNT_C, BANK_C_BG_B),
    };
    w8(&mut m, bank_reg, bank);
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    w16(&mut m, eng.palette, WHITE15);
    LAYER_COLORS.iter().enumerate().for_each(|(n, &color)| {
        let (layer, tile) = (n as u32, n as u32 + 1);
        w16(&mut m, eng.palette + tile * 2, color);
        // Tile `tile`, solid 4-bit colour `tile`, in character block 1; screen block `layer` is a
        // 32 by 32 map of that tile.
        (0..8).for_each(|row| {
            w32(
                &mut m,
                eng.vram + 0x4000 + tile * 32 + row * 4,
                tile * 0x1111_1111,
            );
        });
        (0..512).for_each(|i| w32(&mut m, eng.vram + layer * 0x800 + i * 4, tile | tile << 16));
        w16(&mut m, eng.bg_regs + layer * 2, 1 << 2 | (n as u16) << 8);
    });
    regs.iter()
        .for_each(|&(offset, value)| w16(&mut m, eng.dispcnt + offset, value));
    w32(&mut m, eng.dispcnt, dispcnt);
    m
}

/// The (x, y, got, want) of the probes whose pixel is not the colour wanted, after a frame.
fn misses(
    m: &mut Machine,
    eng: Eng,
    probes: &[(usize, usize, u16)],
) -> Vec<(usize, usize, u32, u32)> {
    m.render_frame();
    probes
        .iter()
        .filter_map(|&(x, y, color)| {
            let (got, want) = (pixel(m, eng.screen, x, y), expand15(color));
            (got != want).then_some((x, y, got, want))
        })
        .collect()
}

/// The six booleans of a layer set: BG0-BG3, OBJ, effect.
fn set_bits(set: LayerSet) -> [bool; 6] {
    [
        set.bg(0),
        set.bg(1),
        set.bg(2),
        set.bg(3),
        set.obj(),
        set.effect(),
    ]
}

// ------------------------------------------------------------------------- the regions

#[test]
fn given_no_window_on_when_drawn_then_every_pixel_shows_the_front_layer_whatever_the_registers_hold()
 {
    // WINOUT hides everything, but with DISPCNT bits 13-15 clear the registers are inert.
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS,
        &[
            (WIN0H, range(64, 192)),
            (WIN0V, range(48, 144)),
            (WINOUT, 0),
        ],
    );
    m.render_frame();
    assert_eq!(
        not_equal_to(&m, Screen::Top, expand15(RED)),
        0,
        "pixels that are not BG0"
    );
}

#[test]
fn given_window_0_when_drawn_then_its_rectangle_shows_winin_and_the_rest_shows_winout() {
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | WIN0_ON,
        &[
            (WIN0H, range(64, 192)),
            (WIN0V, range(48, 144)),
            (WININ, BG1),
            (WINOUT, BG2),
        ],
    );
    // The columns 64..192 and rows 48..144 are inside: the end is one past the last pixel.
    let probes = [
        (63, 100, BLUE),
        (64, 100, GREEN),
        (191, 100, GREEN),
        (192, 100, BLUE),
        (100, 47, BLUE),
        (100, 48, GREEN),
        (100, 143, GREEN),
        (100, 144, BLUE),
        (0, 0, BLUE),
        (255, 191, BLUE),
    ];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_a_layer_hidden_in_a_region_when_drawn_then_the_next_layer_behind_shows() {
    // Inside the window only BG2 and BG3 are on: BG2 is the front of them. Outside, none is on:
    // the backdrop.
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | WIN0_ON,
        &[
            (WIN0H, range(0, 128)),
            (WIN0V, range(0, 192)),
            (WININ, BG2 | BG3),
            (WINOUT, 0),
        ],
    );
    let probes = [(10, 10, BLUE), (127, 191, BLUE), (128, 0, WHITE15)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_a_layer_off_in_dispcnt_when_a_window_enables_it_then_it_still_does_not_show() {
    // DISPCNT bit 8 is clear (BG0 off); the window's BG0 bit alone cannot bring it back.
    let mut m = layers_scene(
        ENG_A,
        (GRAPHICS_BGS & !(1 << 8)) | WIN0_ON,
        &[
            (WIN0H, range(0, 128)),
            (WIN0V, range(0, 192)),
            (WININ, BG0 | BG1),
            (WINOUT, 0x3F),
        ],
    );
    let probes = [(10, 10, GREEN), (200, 10, GREEN)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_overlapping_windows_when_drawn_then_window_0_wins_over_window_1_and_both_over_outside() {
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | WIN0_ON | WIN1_ON,
        &[
            (WIN0H, range(0, 128)),
            (WIN0V, range(0, 192)),
            (WIN1H, range(64, 192)),
            (WIN1V, range(0, 192)),
            (WININ, BG0 | high(BG1)),
            (WINOUT, BG3),
        ],
    );
    // Column 100 is in both windows: window 0's BG0. Column 150 is in window 1 only.
    let probes = [
        (32, 5, RED),
        (100, 5, RED),
        (127, 5, RED),
        (128, 5, GREEN),
        (150, 5, GREEN),
        (191, 5, GREEN),
        (192, 5, YELLOW),
    ];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_only_window_1_on_when_drawn_then_window_0_s_rectangle_is_ignored() {
    // Both rectangles are set, window 0's covers the probe, but only DISPCNT bit 14 is set.
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | WIN1_ON,
        &[
            (WIN0H, range(0, 255)),
            (WIN0V, range(0, 192)),
            (WIN1H, range(100, 120)),
            (WIN1V, range(0, 192)),
            (WININ, BG0 | high(BG1)),
            (WINOUT, BG2),
        ],
    );
    let probes = [(50, 50, BLUE), (110, 50, GREEN), (130, 50, BLUE)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_x1_past_x2_when_drawn_then_the_window_runs_from_x1_to_the_right_edge() {
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | WIN0_ON,
        &[
            (WIN0H, range(200, 100)),
            (WIN0V, range(0, 192)),
            (WININ, BG1),
            (WINOUT, BG2),
        ],
    );
    let probes = [
        (99, 7, BLUE),
        (150, 7, BLUE),
        (199, 7, BLUE),
        (200, 7, GREEN),
        (255, 7, GREEN),
    ];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_y1_past_y2_when_drawn_then_the_window_runs_from_y1_to_the_bottom_edge() {
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | WIN0_ON,
        &[
            (WIN0H, range(0, 255)),
            (WIN0V, range(150, 100)),
            (WININ, BG1),
            (WINOUT, BG2),
        ],
    );
    let probes = [
        (5, 99, BLUE),
        (5, 149, BLUE),
        (5, 150, GREEN),
        (5, 191, GREEN),
    ];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_y2_past_the_screen_when_drawn_then_the_window_runs_to_the_bottom_edge() {
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | WIN0_ON,
        &[
            (WIN0H, range(0, 255)),
            (WIN0V, range(40, 250)),
            (WININ, BG1),
            (WINOUT, BG2),
        ],
    );
    let probes = [(5, 39, BLUE), (5, 40, GREEN), (5, 191, GREEN)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_a_window_with_x1_equal_to_x2_when_drawn_then_it_covers_nothing() {
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | WIN0_ON,
        &[
            (WIN0H, range(100, 100)),
            (WIN0V, range(0, 192)),
            (WININ, BG1),
            (WINOUT, BG2),
        ],
    );
    let probes = [(99, 7, BLUE), (100, 7, BLUE), (101, 7, BLUE)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_a_window_when_a_region_clears_every_layer_then_the_backdrop_shows_there() {
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | WIN0_ON,
        &[
            (WIN0H, range(10, 20)),
            (WIN0V, range(10, 20)),
            (WININ, 0),
            (WINOUT, BG0),
        ],
    );
    let probes = [(15, 15, WHITE15), (25, 15, RED)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

/// The colour the regions rule gives (`x`, `y`) in the model scene below.
fn model_color(x: usize, y: usize) -> u16 {
    let inside = |(x1, x2, y1, y2): (usize, usize, usize, usize)| {
        (x1..x2).contains(&x) && (y1..y2).contains(&y)
    };
    let group = match (inside((30, 120, 20, 100)), inside((90, 200, 60, 160))) {
        (true, _) => BG1 | BG3,
        (false, true) => BG2,
        (false, false) => BG0,
    };
    (0..4)
        .find(|&n| group >> n & 1 != 0)
        .map_or(WHITE15, |n| LAYER_COLORS[n])
}

/// The pixels of a frame that differ from [`model_color`], as (x, y).
fn model_misses(eng: Eng) -> Vec<(usize, usize)> {
    let mut m = layers_scene(
        eng,
        GRAPHICS_BGS | WIN0_ON | WIN1_ON,
        &[
            (WIN0H, range(30, 120)),
            (WIN0V, range(20, 100)),
            (WIN1H, range(90, 200)),
            (WIN1V, range(60, 160)),
            (WININ, BG1 | BG3 | high(BG2)),
            (WINOUT, BG0),
        ],
    );
    m.render_frame();
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .filter(|&(x, y)| pixel(&m, eng.screen, x, y) != expand15(model_color(x, y)))
        .take(8)
        .collect()
}

#[test]
fn given_two_overlapping_windows_when_the_whole_frame_is_drawn_then_every_pixel_follows_the_region_rule()
 {
    assert_eq!(model_misses(ENG_A), vec![]);
}

#[test]
fn given_the_same_windows_on_engine_b_when_drawn_then_the_lower_screen_follows_the_region_rule() {
    assert_eq!(model_misses(ENG_B), vec![]);
}

#[test]
fn given_the_windows_of_engine_a_when_engine_b_has_none_then_engine_b_shows_the_front_layer() {
    let mut m = layers_scene(ENG_B, GRAPHICS_BGS, &[]);
    // Engine A's window registers do not reach engine B.
    w16(&mut m, DISPCNT_A_ADDR + WINOUT, 0);
    w16(&mut m, DISPCNT_A_ADDR + WIN0H, range(0, 255));
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Bottom, expand15(RED)), 0);
}

// ------------------------------------------------------------------------- the objects

/// Bank B as engine A's OBJ VRAM (MST 2): one solid tile 0 of colour 1 and a palette whose entry 1
/// is magenta, with every OAM entry disabled.
fn prepare_objects(m: &mut Machine) {
    w8(m, 0x0400_0241, 0x82);
    w16(m, 0x0500_0202, MAGENTA);
    (0..8).for_each(|row| w32(m, 0x0640_0000 + row * 4, 0x1111_1111));
    (0..128u32).for_each(|entry| w16(m, 0x0700_0000 + entry * 8, 1 << 9));
}

/// Writes entry `entry` of engine A's OAM.
fn put_object(m: &mut Machine, entry: u32, attrs: [u16; 3]) {
    attrs
        .iter()
        .enumerate()
        .for_each(|(n, &half)| w16(m, 0x0700_0000 + entry * 8 + n as u32 * 2, half));
}

/// Two 8 by 8 objects at (10, 10) and (200, 10).
fn put_sprites(m: &mut Machine) {
    prepare_objects(m);
    put_object(m, 0, [10, 10, 0]);
    put_object(m, 1, [10, 200, 0]);
}

#[test]
fn given_the_obj_bit_clear_inside_a_window_when_drawn_then_sprites_there_hide_and_show_outside() {
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | OBJ_ON | WIN0_ON,
        &[
            (WIN0H, range(0, 128)),
            (WIN0V, range(0, 192)),
            (WININ, BG0),
            (WINOUT, BG0 | OBJ),
        ],
    );
    put_sprites(&mut m);
    // A hidden sprite is as if it were not there: BG0 shows where it would have been, and an object
    // outside the window is drawn over the layers.
    let probes = [(12, 12, RED), (203, 12, MAGENTA), (150, 100, RED)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_the_obj_bit_set_inside_a_window_and_clear_outside_when_drawn_then_only_the_inside_sprite_shows()
 {
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | OBJ_ON | WIN0_ON,
        &[
            (WIN0H, range(0, 128)),
            (WIN0V, range(0, 192)),
            (WININ, BG0 | OBJ),
            (WINOUT, BG0),
        ],
    );
    put_sprites(&mut m);
    let probes = [(12, 12, MAGENTA), (203, 12, RED)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

// ------------------------------------------------------------------------- the registers

#[test]
fn given_the_window_registers_when_decoded_then_the_inspect_view_shows_the_clamped_rectangles() {
    let mut m = Machine::new();
    w16(&mut m, DISPCNT_A_ADDR + WIN0H, range(200, 100));
    w16(&mut m, DISPCNT_A_ADDR + WIN0V, range(10, 250));
    w16(&mut m, DISPCNT_A_ADDR + WIN1H, range(5, 77));
    w16(&mut m, DISPCNT_A_ADDR + WIN1V, range(8, 9));
    w16(&mut m, DISPCNT_A_ADDR + WININ, 0x2A15);
    w16(&mut m, DISPCNT_A_ADDR + WINOUT, 0x0C03 | 1 << 5);
    w32(&mut m, DISPCNT_A_ADDR, WIN0_ON | OBJ_WIN_ON);
    let windows = m.inspect().gpu2d().windows(EngineId::A);
    let rects = windows.rects.map(|r| (r.left, r.right, r.top, r.bottom));
    assert_eq!(
        (
            rects,
            windows.enabled,
            windows.inside.map(set_bits),
            set_bits(windows.outside),
            set_bits(windows.obj_window),
        ),
        (
            [(200, 256, 10, 192), (5, 77, 8, 9)],
            [true, false, true],
            [
                [true, false, true, false, true, false],
                [false, true, false, true, false, true],
            ],
            [true, true, false, false, false, true],
            [false, false, true, true, false, false],
        )
    );
}

#[test]
fn given_bytes_written_to_a_rectangle_when_decoded_then_x2_is_the_low_byte_and_x1_the_high_one() {
    let mut m = Machine::new();
    w8(&mut m, DISPCNT_A_ADDR + WIN0H, 100);
    w8(&mut m, DISPCNT_A_ADDR + WIN0H + 1, 40);
    let rect = m.inspect().gpu2d().windows(EngineId::A).rects[0];
    assert_eq!((rect.left, rect.right), (40, 100));
}

#[test]
fn given_winin_and_winout_when_written_then_they_read_back_without_the_unused_bits() {
    let mut m = Machine::new();
    w16(&mut m, DISPCNT_A_ADDR + WININ, 0xFFFF);
    w8(&mut m, DISPCNT_A_ADDR + WINOUT + 1, 0xFF);
    assert_eq!(
        (
            r16(&m, DISPCNT_A_ADDR + WININ),
            r16(&m, DISPCNT_A_ADDR + WINOUT)
        ),
        (0x3F3F, 0x3F00)
    );
}

/// Writes 0x1234 to each rectangle register and reads each back.
fn rect_reads_after_writing(m: &mut Machine) -> [u16; 4] {
    let rects = [WIN0H, WIN1H, WIN0V, WIN1V];
    rects
        .iter()
        .for_each(|&offset| w16(m, DISPCNT_A_ADDR + offset, 0x1234));
    rects.map(|offset| r16(m, DISPCNT_A_ADDR + offset))
}

#[test]
fn given_the_rectangle_registers_when_written_then_they_read_back_zero() {
    let mut m = Machine::new();
    assert_eq!(rect_reads_after_writing(&mut m), [0; 4]);
}

#[test]
fn given_engine_b_powered_off_when_its_window_registers_are_written_then_they_are_ignored() {
    let mut m = Machine::new();
    // POWCNT1 with bit 9 clear.
    w16(&mut m, POWCNT1_ADDR, 0x800F);
    w16(&mut m, DISPCNT_B_ADDR + WININ, 0x1F1F);
    assert_eq!(r16(&m, DISPCNT_B_ADDR + WININ), 0);
}

#[test]
fn given_window_registers_when_a_savestate_is_loaded_then_the_new_machine_has_them() {
    let mut saver = Machine::new();
    w16(&mut saver, DISPCNT_A_ADDR + WIN0H, range(10, 20));
    w16(&mut saver, DISPCNT_B_ADDR + WIN1V, range(30, 40));
    w16(&mut saver, DISPCNT_A_ADDR + WININ, 0x1234);
    w16(&mut saver, DISPCNT_B_ADDR + WINOUT, 0x0A05);
    w32(&mut saver, DISPCNT_B_ADDR, WIN1_ON);
    let mut target = Machine::new();
    let result = target.load_state(&saver.save_state());
    let views = |m: &Machine| {
        let gpu = m.inspect().gpu2d();
        (gpu.windows(EngineId::A), gpu.windows(EngineId::B))
    };
    assert_eq!(
        (result, views(&target)),
        (Ok(()), views(&saver)),
        "the loaded machine differs from the one saved"
    );
}

#[test]
fn given_a_dirty_window_state_when_a_machine_with_it_is_compared_to_a_fresh_one_then_they_differ() {
    // Pins that the registers are part of the state: a savestate of a fresh machine does not equal
    // one with a rectangle written.
    let fresh = Machine::new();
    let mut written = Machine::new();
    w16(&mut written, DISPCNT_A_ADDR + WIN0H, range(1, 2));
    assert_ne!(fresh.save_state(), written.save_state());
}

#[test]
fn given_the_effect_bit_when_set_then_it_is_stored_and_changes_no_pixel() {
    // The colour special effect is #34: the bit reads back and the picture is the same.
    let regs = |winin| {
        [
            (WIN0H, range(0, 128)),
            (WIN0V, range(0, 192)),
            (WININ, winin),
            (WINOUT, BG2),
        ]
    };
    let mut plain = layers_scene(ENG_A, GRAPHICS_BGS | WIN0_ON, &regs(BG1));
    let mut with = layers_scene(ENG_A, GRAPHICS_BGS | WIN0_ON, &regs(BG1 | EFFECT));
    plain.render_frame();
    with.render_frame();
    assert_eq!(
        (
            plain.frame(Screen::Top) == with.frame(Screen::Top),
            set_bits(with.inspect().gpu2d().windows(EngineId::A).inside[0])[5],
        ),
        (true, true)
    );
}

// ------------------------------------------------------------------------- the OBJ window

/// attr0 bits: mode 2 (the OBJ window), affine, mosaic.
const MODE_OBJ_WINDOW: u16 = 2 << 10;
const AFFINE: u16 = 1 << 8;
const MOSAIC: u16 = 1 << 12;

/// A scene whose OBJ window group is BG1 and whose outside group is BG0 and the OBJ layer:
/// objects, window 0 and window 1 off.
fn obj_window_scene(extra: u32, regs: &[(u32, u16)]) -> Machine {
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | OBJ_ON | OBJ_WIN_ON | extra,
        &[&[(WINOUT, BG0 | OBJ | high(BG1))], regs].concat(),
    );
    prepare_objects(&mut m);
    m
}

/// Fills each row of OBJ tile 0 with `word`.
fn fill_tile_0(m: &mut Machine, word: u32) {
    (0..8).for_each(|row| w32(m, 0x0640_0000 + row * 4, word));
}

/// Affine group 0: PA, PB, PC and PD, the fourth halfword of entries 0-3 (disabled objects).
fn put_group_0(m: &mut Machine, [pa, pb, pc, pd]: [u16; 4]) {
    [pa, pb, pc, pd]
        .iter()
        .enumerate()
        .for_each(|(n, &value)| w16(m, 0x0700_0000 + n as u32 * 8 + 6, value));
}

#[test]
fn given_a_mode_2_object_when_drawn_then_its_pixels_show_the_obj_window_layers_and_it_draws_no_colour()
 {
    let mut m = obj_window_scene(0, &[]);
    put_object(&mut m, 0, [50 | MODE_OBJ_WINDOW, 50, 0]);
    // The object covers columns 50-57 and rows 50-57; BG1 shows there, BG0 elsewhere, and no pixel is
    // the object's magenta.
    let probes = [
        (50, 50, GREEN),
        (57, 57, GREEN),
        (49, 53, RED),
        (58, 53, RED),
        (53, 49, RED),
        (53, 58, RED),
    ];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_a_mode_2_object_when_no_window_is_on_then_it_draws_nothing() {
    let mut m = layers_scene(ENG_A, GRAPHICS_BGS | OBJ_ON, &[]);
    prepare_objects(&mut m);
    put_object(&mut m, 0, [50 | MODE_OBJ_WINDOW, 50, 0]);
    let probes = [(53, 53, RED), (60, 60, RED)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_a_mode_2_object_with_transparent_pixels_when_drawn_then_only_its_opaque_pixels_are_window()
{
    let mut m = obj_window_scene(0, &[]);
    // Tile 0 keeps colour 1 in its left four columns (the low nibbles) and is transparent right of
    // them.
    fill_tile_0(&mut m, 0x0000_1111);
    put_object(&mut m, 0, [50 | MODE_OBJ_WINDOW, 50, 0]);
    let probes = [
        (50, 52, GREEN),
        (53, 52, GREEN),
        (54, 52, RED),
        (57, 52, RED),
    ];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_window_0_over_the_obj_window_when_drawn_then_window_0_s_layers_show() {
    let mut m = obj_window_scene(
        WIN0_ON,
        &[(WIN0H, range(40, 54)), (WIN0V, range(40, 60)), (WININ, BG2)],
    );
    put_object(&mut m, 0, [50 | MODE_OBJ_WINDOW, 50, 0]);
    // Columns 50-53 are in window 0 and the OBJ window (window 0 wins); columns 54-57 are in the
    // OBJ window alone.
    let probes = [
        (52, 52, BLUE),
        (56, 52, GREEN),
        (45, 45, BLUE),
        (60, 52, RED),
    ];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_window_1_over_the_obj_window_when_drawn_then_window_1_s_layers_show() {
    let mut m = obj_window_scene(
        WIN1_ON,
        &[
            (WIN1H, range(40, 54)),
            (WIN1V, range(40, 60)),
            (WININ, high(BG3)),
        ],
    );
    put_object(&mut m, 0, [50 | MODE_OBJ_WINDOW, 50, 0]);
    let probes = [(52, 52, YELLOW), (56, 52, GREEN), (60, 52, RED)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_a_sprite_over_the_obj_window_when_the_obj_bit_is_clear_there_then_the_sprite_is_hidden() {
    let mut m = obj_window_scene(0, &[]);
    // The window object covers 50-57; a normal object covers 54-61. Inside the window the group is
    // BG1 alone, so the sprite is hidden there; outside, the group has OBJ and shows it.
    put_object(&mut m, 0, [50 | MODE_OBJ_WINDOW, 50, 0]);
    put_object(&mut m, 1, [54, 54, 0]);
    let probes = [(56, 56, GREEN), (60, 60, MAGENTA)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_a_sprite_over_the_obj_window_when_the_obj_bit_is_set_there_then_the_sprite_shows() {
    let mut m = obj_window_scene(0, &[(WINOUT, BG0 | high(BG1 | OBJ))]);
    put_object(&mut m, 0, [50 | MODE_OBJ_WINDOW, 50, 0]);
    put_object(&mut m, 1, [54, 54, 0]);
    // Outside the window the group is BG0 alone, so the sprite is hidden there.
    let probes = [(56, 56, MAGENTA), (60, 60, RED)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_the_obj_layer_off_when_a_mode_2_object_is_listed_then_there_is_no_obj_window() {
    // DISPCNT bit 12 is clear: no object is processed, so no pixel is marked (`windows.md`).
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | OBJ_WIN_ON,
        &[(WINOUT, BG0 | high(BG1))],
    );
    prepare_objects(&mut m);
    put_object(&mut m, 0, [50 | MODE_OBJ_WINDOW, 50, 0]);
    let probes = [(53, 53, RED)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_an_obj_window_bit_clear_when_a_mode_2_object_is_listed_then_the_outside_layers_show() {
    // DISPCNT bit 15 is clear (OBJ window off) but window 0 is on elsewhere: the object's pixels are
    // outside every window.
    let mut m = layers_scene(
        ENG_A,
        GRAPHICS_BGS | OBJ_ON | WIN0_ON,
        &[
            (WIN0H, range(0, 10)),
            (WIN0V, range(0, 10)),
            (WINOUT, BG0 | high(BG1)),
        ],
    );
    prepare_objects(&mut m);
    put_object(&mut m, 0, [50 | MODE_OBJ_WINDOW, 50, 0]);
    let probes = [(53, 53, RED)];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_an_affine_mode_2_object_when_drawn_then_the_mask_follows_the_transform() {
    let mut m = obj_window_scene(0, &[]);
    // Group 0: PA = PD = 2.0 samples twice as far from the centre, so the picture covers the middle
    // four columns and rows of the 8 by 8 box (columns and rows 52-55).
    put_group_0(&mut m, [0x200, 0, 0, 0x200]);
    put_object(&mut m, 4, [50 | AFFINE | MODE_OBJ_WINDOW, 50, 0]);
    let probes = [
        (52, 52, GREEN),
        (55, 55, GREEN),
        (51, 52, RED),
        (56, 52, RED),
        (52, 56, RED),
    ];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

#[test]
fn given_a_mosaic_mode_2_object_when_drawn_then_the_mask_is_made_of_blocks() {
    let mut m = obj_window_scene(0, &[]);
    // OBJ mosaic 4 wide, 1 high. Blocks are counted from the left of the screen: the block at 48-51
    // starts left of the object and is transparent, the blocks 52-55 and 56-59 show it, the block 60-63
    // starts right of its box.
    w16(&mut m, MOSAIC_ADDR, 0x0300);
    put_object(&mut m, 0, [50 | MOSAIC | MODE_OBJ_WINDOW, 50, 0]);
    let probes = [
        (51, 53, RED),
        (52, 53, GREEN),
        (59, 53, GREEN),
        (60, 53, RED),
    ];
    assert_eq!(misses(&mut m, ENG_A, &probes), vec![]);
}

/// The budget scene: `costly` objects that cost 64 cycles each, off the left of the screen on line
/// 40, then a mode 2 object of 8 cycles at column 100.
fn budget_scene(costly: u32) -> Machine {
    let mut m = obj_window_scene(0, &[]);
    (0..costly).for_each(|entry| {
        // A 64 by 32 plain object at X = -256 (entirely off the screen).
        put_object(&mut m, entry, [40 | 1 << 14, 0x100 | 3 << 14, 0]);
    });
    put_object(&mut m, costly, [40 | MODE_OBJ_WINDOW, 100, 0]);
    m
}

#[test]
fn given_18_costly_objects_when_a_mode_2_object_follows_then_it_fits_the_budget_and_marks_its_mask()
{
    // 18 * 64 + 8 = 1160 cycles, within 1210.
    let mut m = budget_scene(18);
    assert_eq!(misses(&mut m, ENG_A, &[(103, 43, GREEN)]), vec![]);
}

#[test]
fn given_19_costly_objects_when_a_mode_2_object_follows_then_the_budget_is_spent_and_it_marks_nothing()
 {
    // The 19th costly object would need 1216 > 1210 cycles: it and every later object are dropped.
    let mut m = budget_scene(19);
    assert_eq!(misses(&mut m, ENG_A, &[(103, 43, RED)]), vec![]);
}
