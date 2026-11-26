// SPDX-License-Identifier: Apache-2.0
//! The rear bitmaps (DISP3DCNT bit 14) through the rasterizer, worked out by hand from
//! `docs/hw/textures.md` ("The rear bitmap"): colour and alpha from slot 2, depth from slot 3, the
//! CLRIMAGE_OFFSET scroll with its wrap, the polygon ID from CLEAR_COLOR, and what an unmapped slot
//! reads.
//!
//! The bitmaps of these tests hold, at entry `(x, y)`: colour red `x / 8`, green `y / 8`, alpha bit
//! set where bit 5 of `x` and of `y` are equal (so tiles of 32x32 alternate solid and transparent);
//! depth 7FFFh (far) for `x` below 128 and 0 (nearest) from 128 on.

use super::texture::*;
use super::*;

/// DISP3DCNT bit 14: the rear plane is the bitmaps; bit 3: blending.
const REAR_BITMAP: u16 = 1 << 14;
const BLENDING: u16 = 1 << 3;
/// Image offsets of the colour bitmap (slot 2) and the depth bitmap (slot 3).
const COLOR_AT: usize = 0x4_0000;
const DEPTH_AT: usize = 0x6_0000;

/// The colour entry at `(x, y)`.
fn color_entry(x: usize, y: usize) -> u16 {
    let solid = ((x >> 5) ^ (y >> 5)) & 1 == 0;
    ((x >> 3) as u16) | (((y >> 3) as u16) << 5) | [0, 0x8000][usize::from(solid)]
}

/// The depth entry at `(x, y)`: far on the left half, nearest on the right half.
fn depth_entry(x: usize, _y: usize) -> u16 {
    [0x7FFF, 0][usize::from(x >= 128)]
}

/// Texture memory holding the two bitmaps.
fn bitmaps() -> TexMem {
    let entries = |f: fn(usize, usize) -> u16| -> Vec<u8> {
        (0..256 * 256)
            .flat_map(|i| f(i % 256, i / 256).to_le_bytes())
            .collect()
    };
    TexMem::new()
        .image(COLOR_AT, &entries(color_entry))
        .image(DEPTH_AT, &entries(depth_entry))
}

/// Registers with the scroll `(x, y)` in CLRIMAGE_OFFSET and a CLEAR_COLOR of ID 9 and colour white
/// at alpha 31 (which the bitmap must replace).
fn scrolled(x: u16, y: u16) -> RenderRegs {
    RenderRegs {
        clrimage_offset: x | y << 8,
        clear_color: 0x7FFF | 31 << 16 | 9 << 24,
        ..regs()
    }
}

/// All lines of `ram` with the rear bitmap on over texture memory `mem`.
fn draw_rear(ram: &Ram, disp3dcnt: u16, mem: &TexMem, regs: &RenderRegs) -> Vec<Line3d> {
    let frame = Frame::new(Scene { ram, swap_bits: 0 }, regs, disp3dcnt | REAR_BITMAP);
    let mut lines = vec![[Pixel3d::default(); SCREEN_WIDTH]; crate::SCREEN_HEIGHT];
    frame.draw(&SequentialExecutor, &mem.view(), &mut lines);
    lines
}

/// An empty scene.
fn nothing() -> Ram {
    scene(&[])
}

#[test]
fn given_the_rear_bitmap_when_nothing_is_drawn_then_each_pixel_has_its_entrys_colour_and_alpha() {
    let lines = draw_rear(&nothing(), 0, &bitmaps(), &scrolled(0, 0));
    // (3, 5) is in a solid tile, (35, 5) in a transparent one, (35, 40) in a solid one again.
    let pixels = [(3, 5), (35, 5), (35, 40)].map(|(x, y)| at(&lines, x, y));
    // Entry (3, 5): red 0, green 0, solid: colour 0. Entry (35, 5): red 4, green 0, transparent.
    // Entry (35, 40): red 4, green 5: 4 + 5 x 32 = 164, solid.
    let want = [(0, 31), (4, 0), (164, 31)];
    assert_eq!(pixels.map(|p| (p.color, p.alpha)), want);
}

#[test]
fn given_a_scroll_when_drawn_then_the_picture_shows_the_bitmap_moved_by_x_and_y() {
    // Scroll (10, 100): screen (0, 0) is entry (10, 100), screen (250, 0) wraps to (4, 100) and
    // screen (0, 190) wraps to (10, 34).
    let lines = draw_rear(&nothing(), 0, &bitmaps(), &scrolled(10, 100));
    let got = [(0, 0), (250, 0), (0, 190)].map(|(x, y)| {
        let p = at(&lines, x, y);
        (p.color, p.alpha)
    });
    let entry = |x: usize, y: usize| {
        (
            color_entry(x, y) & 0x7FFF,
            [0, 31][usize::from(color_entry(x, y) >> 15)],
        )
    };
    assert_eq!(got, [entry(10, 100), entry(4, 100), entry(10, 34)]);
}

#[test]
fn given_the_depth_bitmap_when_a_quad_draws_over_it_then_it_shows_only_where_the_rear_is_far() {
    // The quad at depth z 0 is 7FFFFFh; the rear is FFFFFFh for columns below 128 and 0 from 128 on.
    let ram = textured(&[(opaque(1), 0, 0, quad(112, 48, 144, 96, 0, RED))]);
    let lines = draw_rear(&ram, 0, &bitmaps(), &scrolled(0, 0));
    let got = [(120, 60), (136, 60)].map(|(x, y)| at(&lines, x, y).color);
    assert_eq!(got, [RED, color_entry(136, 60) & 0x7FFF]);
}

#[test]
fn given_a_rear_depth_next_to_the_quads_depth_when_it_is_drawn_then_the_depth_expands_as_clear_depth_does()
 {
    // 4000h expands to 800000h, just behind the quad's 7FFFFFh; 3FFFh expands to 7FFE00h, in front.
    let mem = TexMem::new()
        .image(COLOR_AT, &[0xFF; 0x2_0000])
        .image(DEPTH_AT, &[0x00, 0x40, 0xFF, 0x3F]);
    let ram = textured(&[(opaque(1), 0, 0, quad(0, 0, 2, 1, 0, RED))]);
    let lines = draw_rear(&ram, 0, &mem, &scrolled(0, 0));
    let got = [at(&lines, 0, 0).color, at(&lines, 1, 0).color];
    assert_eq!(got, [RED, 0x7FFF]);
}

#[test]
fn given_the_bitmap_bit_clear_when_slots_2_and_3_hold_data_then_clear_color_is_the_rear_plane() {
    let frame = Frame::new(
        Scene {
            ram: &nothing(),
            swap_bits: 0,
        },
        &scrolled(0, 0),
        0,
    );
    let mut lines = vec![[Pixel3d::default(); SCREEN_WIDTH]; crate::SCREEN_HEIGHT];
    frame.draw(&SequentialExecutor, &bitmaps().view(), &mut lines);
    assert_eq!(
        (at(&lines, 3, 5).color, at(&lines, 3, 5).alpha),
        (0x7FFF, 31)
    );
}

#[test]
fn given_an_unmapped_colour_slot_when_drawn_then_the_rear_is_transparent_colour_0() {
    let lines = draw_rear(&nothing(), 0, &bitmaps().without_slot(2), &scrolled(0, 0));
    assert_eq!((at(&lines, 3, 5).color, at(&lines, 3, 5).alpha), (0, 0));
}

#[test]
fn given_an_unmapped_depth_slot_when_a_quad_draws_then_nothing_passes_the_nearest_rear_depth() {
    let ram = textured(&[(opaque(1), 0, 0, quad(16, 16, 24, 24, -4000, RED))]);
    let lines = draw_rear(&ram, 0, &bitmaps().without_slot(3), &scrolled(0, 0));
    assert_eq!(at(&lines, 20, 20).color, color_entry(20, 20) & 0x7FFF);
}

#[test]
fn given_a_translucent_quad_when_it_draws_over_a_solid_rear_pixel_then_it_blends_with_the_bitmap_colour()
 {
    // Rear entry (3, 5) is solid with red 0, green 0: black. A green quad of alpha 15 over it mixes
    // (new x 16 + old x 16) / 32 per 6-bit channel: green 63 -> 31, narrowed to 15.
    let attr = 15 << 16 | FRONT_BACK;
    let ram = textured(&[(attr, 0, 0, quad(0, 0, 8, 8, 0, GREEN))]);
    let lines = draw_rear(&ram, BLENDING, &bitmaps(), &scrolled(0, 0));
    assert_eq!(at(&lines, 3, 5).color, 15 << 5);
}
