// SPDX-License-Identifier: Apache-2.0
//! Palette textures through the rasterizer, worked out by hand from `docs/hw/textures.md`: the
//! texel layouts of the 4-, 16- and 256-colour formats, PLTT_BASE units, colour 0, repeat, flip and
//! clamp, modulation with the vertex colour, and what an unmapped slot reads.
//!
//! A white vertex colour (63 in 6 bits) makes modulation the identity: `((t + 1) x 64 - 1) / 64 = t`,
//! so a pixel shows its texel's colour. A quad that is 8 pixels wide with S running 0 to 8 texels
//! samples one texel per pixel, at the pixel's left edge.

use super::*;
use crate::gpu3d::polygon::Ram;
use crate::gpu3d::raster::texture::{Texel, modulate, wrap};

const WHITE: u16 = 0x7FFF;
/// DISP3DCNT bit 0: texture mapping on.
const TEXTURES_ON: u16 = 1;
// TEXIMAGE_PARAM bits (`textures.md`).
const REPEAT_S: u32 = 1 << 16;
const REPEAT_T: u32 = 1 << 17;
const FLIP_S: u32 = 1 << 18;
const FLIP_T: u32 = 1 << 19;
const COLOR0_CLEAR: u32 = 1 << 29;
const COLORS4: u32 = 2;
const COLORS16: u32 = 3;
const COLORS256: u32 = 4;
/// Where the test's texture memory puts palette slot 0 in its buffer: after the 4 image slots.
const PALETTE_AT: usize = 0x8_0000;

/// TEXIMAGE_PARAM for a texture at byte offset `offset`, `8 << size` texels each way, of `format`,
/// with `flags` (repeat, flip, colour 0).
fn param(offset: u32, size: u32, format: u32, flags: u32) -> u32 {
    (offset / 8) | (size << 20) | (size << 23) | (format << 26) | flags
}

/// The texture memory of a test: the four image slots at 0, 128 KiB, ..., the six palette slots
/// after them, and the image slots in `unmapped` backed by no bank.
struct TexMem {
    vram: Vec<u8>,
    unmapped: [bool; 4],
}

impl TexMem {
    fn new() -> Self {
        Self {
            vram: vec![0; PALETTE_AT + 6 * 0x4000],
            unmapped: [false; 4],
        }
    }

    fn image(mut self, offset: usize, bytes: &[u8]) -> Self {
        self.vram[offset..offset + bytes.len()].copy_from_slice(bytes);
        self
    }

    fn palette(mut self, offset: usize, colors: &[u16]) -> Self {
        colors.iter().enumerate().for_each(|(k, c)| {
            let at = PALETTE_AT + offset + 2 * k;
            self.vram[at..at + 2].copy_from_slice(&c.to_le_bytes());
        });
        self
    }

    fn without_slot(mut self, slot: usize) -> Self {
        self.unmapped[slot] = true;
        self
    }

    fn view(&self) -> Textures<'_> {
        let image = [0, 1, 2, 3].map(|n| (!self.unmapped[n]).then_some(n as u32 * 0x2_0000));
        let palette = [0, 1, 2, 3, 4, 5].map(|n| Some(PALETTE_AT as u32 + n * 0x4000));
        Textures::from_slots(&self.vram, image, palette)
    }
}

/// A distinct 15-bit colour for each palette index.
fn colour(k: u16) -> u16 {
    (k & 31) | (((k * 3) & 31) << 5) | (((k * 7) & 31) << 10)
}

/// The colours `colour(0)`, `colour(1)`, ... for `n` palette entries.
fn palette_of(n: u16) -> Vec<u16> {
    (0..n).map(colour).collect()
}

/// A quad from `(x0, y0)` to `(x1, y1)` whose S and T run from `(s0, t0)` to `(s1, t1)` texels,
/// white, at depth `z`.
fn tquad(rect: (i32, i32, i32, i32), z: i32, st: (i16, i16, i16, i16)) -> Vec<Vertex> {
    let (x0, y0, x1, y1) = rect;
    let (s0, t0, s1, t1) = st;
    let at = |x, y, s, t| Vertex {
        s: s * 16,
        t: t * 16,
        ..vertex(x, y, z, WHITE)
    };
    vec![
        at(x0, y0, s0, t0),
        at(x0, y1, s0, t1),
        at(x1, y1, s1, t1),
        at(x1, y0, s1, t0),
    ]
}

/// The unit quad of most tests: pixels 16..24 on both axes, one texel per pixel.
fn unit_quad() -> Vec<Vertex> {
    tquad((16, 16, 24, 24), 0, (0, 0, 8, 8))
}

/// A scene of textured polygons `(attr, TEXIMAGE_PARAM, PLTT_BASE, vertices)`.
fn textured(polygons: &[(u32, u32, u32, Vec<Vertex>)]) -> Ram {
    let plain: Vec<(u32, Vec<Vertex>)> = polygons.iter().map(|p| (p.0, p.3.clone())).collect();
    let mut ram = scene(&plain);
    polygons.iter().enumerate().for_each(|(k, p)| {
        ram.polygons[k].texture = p.1;
        ram.polygons[k].palette = p.2;
    });
    ram
}

/// All lines of `ram` with texture mapping on, drawn over texture memory `mem`.
fn draw_with(ram: &Ram, disp3dcnt: u16, mem: &TexMem) -> Vec<Line3d> {
    let frame = Frame::new(Scene { ram, swap_bits: 0 }, &regs(), disp3dcnt);
    let mut lines = vec![[Pixel3d::default(); SCREEN_WIDTH]; crate::SCREEN_HEIGHT];
    frame.draw(&SequentialExecutor, &mem.view(), &mut lines);
    lines
}

/// The pixels of the 8x8 block at `(16, 16)` that do not show `want[y][x]` at alpha 31:
/// `(x, y, colour, alpha)` of each.
fn block_diffs(lines: &[Line3d], want: &[[u16; 8]; 8]) -> Vec<(usize, usize, u16, u8)> {
    (0..64)
        .filter(|k| {
            let p = at(lines, 16 + k % 8, 16 + k / 8);
            (p.color, p.alpha) != (want[k / 8][k % 8], 31)
        })
        .map(|k| {
            let p = at(lines, 16 + k % 8, 16 + k / 8);
            (k % 8, k / 8, p.color, p.alpha)
        })
        .collect()
}

/// The 8x8 grid of `f(x, y)`.
fn grid(f: impl Fn(u16, u16) -> u16) -> [[u16; 8]; 8] {
    std::array::from_fn(|y| std::array::from_fn(|x| f(x as u16, y as u16)))
}

/// A colour for palette index `k` of up to 255 that differs in every index: red `k mod 32`, green
/// `k / 32`.
fn wide(k: u16) -> u16 {
    (k & 31) | ((k >> 5) << 5)
}

#[test]
fn given_a_256_color_texture_when_one_texel_maps_to_a_pixel_then_each_pixel_shows_its_palette_colour()
 {
    // Texel i has index 4i + 3, up to 255, so a lost high bit would show.
    let bytes: Vec<u8> = (0..64u8).map(|i| 4 * i + 3).collect();
    let palette: Vec<u16> = (0..256).map(wide).collect();
    let mem = TexMem::new().image(0, &bytes).palette(0, &palette);
    let ram = textured(&[(opaque(1), param(0, 0, COLORS256, 0), 0, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    assert_eq!(
        block_diffs(&lines, &grid(|x, y| wide(4 * (y * 8 + x) + 3))),
        vec![]
    );
}

#[test]
fn given_a_16_color_texture_when_drawn_then_the_low_nibble_is_the_left_texel() {
    // Texel i has index i mod 16; byte j holds texels 2j (low nibble) and 2j + 1 (high nibble).
    let bytes: Vec<u8> = (0..32u8)
        .map(|j| ((2 * j) % 16) | (((2 * j + 1) % 16) << 4))
        .collect();
    let mem = TexMem::new().image(0, &bytes).palette(0, &palette_of(16));
    let ram = textured(&[(opaque(1), param(0, 0, COLORS16, 0), 0, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    assert_eq!(
        block_diffs(&lines, &grid(|x, y| colour((y * 8 + x) % 16))),
        vec![]
    );
}

#[test]
fn given_a_4_color_texture_when_drawn_then_the_low_bits_are_the_left_texel() {
    // Texel (x, y) has index (x + y) mod 4; byte j holds texels 4j (bits 0-1) to 4j + 3 (bits 6-7).
    let index = |i: u16| ((i / 8) + (i % 8)) % 4;
    let bytes: Vec<u8> = (0..16u16)
        .map(|j| (0..4).fold(0, |b, k| b | (index(4 * j + k) as u8) << (2 * k)))
        .collect();
    let mem = TexMem::new().image(0, &bytes).palette(0, &palette_of(4));
    let ram = textured(&[(opaque(1), param(0, 0, COLORS4, 0), 0, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    assert_eq!(
        block_diffs(&lines, &grid(|x, y| colour((x + y) % 4))),
        vec![]
    );
}

#[test]
fn given_pltt_base_1_when_a_4_color_and_a_16_color_texture_draw_then_the_bases_are_8_and_16_bytes()
{
    // One texel of each, index 3, with the palette entries at byte 8 + 6 (4-colour) and 16 + 6
    // (16-colour): the same PLTT_BASE of 1 is a different byte offset in each format.
    let (c4, c16) = (colour(9), colour(17));
    let mut mem = TexMem::new()
        .image(0, &[0xFF; 16])
        .image(0x100, &[0x33; 32]);
    mem = mem.palette(8 + 6, &[c4]).palette(16 + 6, &[c16]);
    let ram = textured(&[
        (opaque(1), param(0, 0, COLORS4, 0), 1, unit_quad()),
        (
            opaque(2),
            param(0x100, 0, COLORS16, 0),
            1,
            tquad((32, 16, 40, 24), 0, (0, 0, 8, 8)),
        ),
    ]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    let got = (at(&lines, 16, 16).color, at(&lines, 32, 16).color);
    assert_eq!(got, (c4, c16));
}

/// The alpha and colour of the first pixel of a texture of one texel value 0 with colour 0 of the
/// palette being `colour(5)`, with TEXIMAGE_PARAM flags `flags`.
fn index_zero_pixel(flags: u32) -> (u8, u16) {
    let mem = TexMem::new().palette(0, &[colour(5)]);
    let ram = textured(&[(opaque(1), param(0, 0, COLORS256, flags), 0, unit_quad())]);
    let p = at(&draw_with(&ram, TEXTURES_ON, &mem), 16, 16);
    (p.alpha, p.color)
}

#[test]
fn given_the_colour_0_bit_clear_when_index_0_is_read_then_the_palette_colour_shows_at_full_alpha() {
    assert_eq!(index_zero_pixel(0), (31, colour(5)));
}

#[test]
fn given_the_colour_0_bit_set_when_index_0_is_read_then_the_pixel_is_transparent() {
    assert_eq!(index_zero_pixel(COLOR0_CLEAR).0, 0);
}

/// The cases `(n, size, repeat, flip, texel)` of [`wrap`] that give another texel.
fn wrap_failures() -> Vec<(i64, bool, bool, u32, u32)> {
    let cases: [(i64, bool, bool, u32); 24] = [
        // Clamp: the edge texels repeat outwards; flip needs repeat, so it changes nothing.
        (-3, false, false, 0),
        (0, false, false, 0),
        (7, false, false, 7),
        (8, false, false, 7),
        (100, false, true, 7),
        // Repeat: modulo the size, so a negative coordinate wraps from the far edge.
        (-1, true, false, 7),
        (-9, true, false, 7),
        (-8, true, false, 0),
        (8, true, false, 0),
        (9, true, false, 1),
        (15, true, false, 7),
        // Repeat and flip: every second repetition is mirrored.
        (7, true, true, 7),
        (8, true, true, 7),
        (9, true, true, 6),
        (15, true, true, 0),
        (16, true, true, 0),
        (17, true, true, 1),
        (23, true, true, 7),
        (24, true, true, 7),
        (-1, true, true, 0),
        (-2, true, true, 1),
        (-8, true, true, 7),
        (-9, true, true, 7),
        (-16, true, true, 0),
    ];
    cases
        .iter()
        .map(|&(n, repeat, flip, want)| (n, repeat, flip, wrap(n, 8, repeat, flip), want))
        .filter(|(.., got, want)| got != want)
        .collect()
}

#[test]
fn given_texel_coordinates_at_and_beyond_the_size_when_wrapped_then_clamp_repeat_and_flip_agree_with_the_note()
 {
    assert_eq!(wrap_failures(), vec![]);
}

/// The palette index each pixel of line 19 (texture row 3) reads across a 24-pixel quad whose S runs
/// from -8 to 16 texels, under TEXIMAGE_PARAM flags `flags`, as the 8x8 texture has texel index
/// `y x 8 + x`.
fn wrapped_row(flags: u32) -> Vec<u16> {
    let bytes: Vec<u8> = (0..64).collect();
    let mem = TexMem::new().image(0, &bytes).palette(0, &palette_of(64));
    let quad = tquad((16, 16, 40, 24), 0, (-8, 0, 16, 8));
    let ram = textured(&[(opaque(1), param(0, 0, COLORS256, flags), 0, quad)]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    (16..40).map(|x| at(&lines, x, 19).color).collect()
}

/// The colours of texture row 3 at the given columns.
fn row3(columns: &[u16]) -> Vec<u16> {
    columns.iter().map(|c| colour(24 + c)).collect()
}

#[test]
fn given_clamp_when_s_runs_past_both_edges_then_the_edge_texels_repeat() {
    let columns = [[0; 9].as_slice(), &[1, 2, 3, 4, 5, 6, 7], &[7; 8]].concat();
    assert_eq!(wrapped_row(0), row3(&columns));
}

#[test]
fn given_repeat_when_s_runs_from_minus_8_to_16_then_the_texture_tiles_three_times() {
    let columns = [0, 1, 2, 3, 4, 5, 6, 7].repeat(3);
    assert_eq!(wrapped_row(REPEAT_S | REPEAT_T), row3(&columns));
}

#[test]
fn given_repeat_and_flip_when_s_runs_from_minus_8_to_16_then_every_second_tile_mirrors() {
    let columns = [
        [7, 6, 5, 4, 3, 2, 1, 0],
        [0, 1, 2, 3, 4, 5, 6, 7],
        [7, 6, 5, 4, 3, 2, 1, 0],
    ]
    .concat();
    assert_eq!(
        wrapped_row(REPEAT_S | REPEAT_T | FLIP_S | FLIP_T),
        row3(&columns)
    );
}

#[test]
fn given_repeat_in_t_only_when_t_runs_past_the_bottom_then_the_rows_wrap() {
    // T from 4 to 12 over the 8 rows: rows 4..7 then 0..3 (the texture's T repeats, S is in range).
    let bytes: Vec<u8> = (0..64).collect();
    let mem = TexMem::new().image(0, &bytes).palette(0, &palette_of(64));
    let quad = tquad((16, 16, 24, 24), 0, (0, 4, 8, 12));
    let ram = textured(&[(opaque(1), param(0, 0, COLORS256, REPEAT_T), 0, quad)]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    let want = grid(|x, y| colour((y + 4) % 8 * 8 + x));
    assert_eq!(block_diffs(&lines, &want), vec![]);
}

/// A 256-colour 8x8 texture of index 1 at image offset `0x1FFE0`, which crosses from slot 0 into
/// slot 1 after 32 bytes (rows 0 to 3), with colour 0 transparent and `memory` as texture memory.
fn straddling(memory: TexMem) -> Vec<Line3d> {
    let mem = memory
        .image(0x1_FFE0, &[1; 32])
        .image(0x2_0000, &[1; 32])
        .palette(0, &palette_of(2));
    let ram = textured(&[(
        opaque(1),
        param(0x1_FFE0, 0, COLORS256, COLOR0_CLEAR),
        0,
        unit_quad(),
    )]);
    draw_with(&ram, TEXTURES_ON, &mem)
}

#[test]
fn given_a_texture_that_straddles_into_a_mapped_slot_when_drawn_then_every_row_shows_its_texels() {
    let lines = straddling(TexMem::new());
    assert_eq!(block_diffs(&lines, &grid(|_, _| colour(1))), vec![]);
}

#[test]
fn given_a_texture_that_straddles_into_an_unmapped_slot_when_drawn_then_the_unbacked_rows_are_index_0()
 {
    // Rows 0 to 3 are in slot 0; rows 4 to 7 read 0 and, with colour 0 transparent, are hidden.
    let lines = straddling(TexMem::new().without_slot(1));
    let alphas: Vec<u8> = (16..24).map(|y| at(&lines, 20, y).alpha).collect();
    assert_eq!(alphas, vec![31, 31, 31, 31, 0, 0, 0, 0]);
}

/// A white 5-bit channel `t` modulated by a vertex channel `v`, both as 5-bit values: the 6-bit
/// result.
fn modulated(t: u16, v: u16) -> u8 {
    let texel = Texel {
        color: t,
        alpha: 31,
    };
    let (color, _) = modulate(texel, [crate::gpu3d::raster::texture::expand6(v); 3], 31);
    color[0]
}

#[test]
fn given_texel_and_vertex_channels_when_modulated_then_the_result_is_the_6_bit_product() {
    // ((t6 + 1) x (v6 + 1) - 1) / 64 with t6 = 2t + 1 and v6 = 2v + 1, worked by hand.
    let cases: [(u16, u16, u8); 6] = [
        (31, 31, 63), // 64 x 64 - 1 = 4095, / 64 = 63
        (31, 16, 33), // 64 x 34 - 1 = 2175, / 64 = 33
        (16, 16, 18), // 34 x 34 - 1 = 1155, / 64 = 18
        (8, 31, 17),  // 18 x 64 - 1 = 1151, / 64 = 17
        (0, 31, 0),   // 1 x 64 - 1 = 63, / 64 = 0
        (31, 0, 0),   // 64 x 1 - 1 = 63, / 64 = 0
    ];
    let wrong: Vec<(u16, u16, u8, u8)> = cases
        .iter()
        .map(|&(t, v, want)| (t, v, modulated(t, v), want))
        .filter(|(_, _, got, want)| got != want)
        .collect();
    assert_eq!(wrong, vec![]);
}

#[test]
fn given_a_grey_vertex_colour_when_a_white_texel_is_drawn_then_the_pixel_is_the_modulated_grey() {
    // Vertex channels 16 (6-bit 33) times a white texel (63): 33, narrowed to 16 in 5 bits.
    let mem = TexMem::new().image(0, &[1]).palette(0, &[0, WHITE]);
    let grey = 16 | 16 << 5 | 16 << 10;
    let vs: Vec<Vertex> = unit_quad()
        .into_iter()
        .map(|v| Vertex { color: grey, ..v })
        .collect();
    let ram = textured(&[(
        opaque(1),
        param(0, 0, COLORS256, REPEAT_S | REPEAT_T),
        0,
        vs,
    )]);
    let p = at(&draw_with(&ram, TEXTURES_ON, &mem), 16, 16);
    assert_eq!(p.color, grey);
}

#[test]
fn given_texturing_off_in_disp3dcnt_when_a_textured_polygon_draws_then_it_shows_its_vertex_colour()
{
    let mem = TexMem::new()
        .image(0, &[1])
        .palette(0, &[colour(2), colour(3)]);
    let vs: Vec<Vertex> = unit_quad()
        .into_iter()
        .map(|v| Vertex { color: RED, ..v })
        .collect();
    let ram = textured(&[(opaque(1), param(0, 0, COLORS256, 0), 0, vs)]);
    assert_eq!(at(&draw_with(&ram, 0, &mem), 20, 20).color, RED);
}

#[test]
fn given_texture_format_0_when_a_polygon_draws_then_it_shows_its_vertex_colour() {
    let mem = TexMem::new()
        .image(0, &[1])
        .palette(0, &[colour(2), colour(3)]);
    let vs: Vec<Vertex> = unit_quad()
        .into_iter()
        .map(|v| Vertex { color: GREEN, ..v })
        .collect();
    let ram = textured(&[(opaque(1), param(0, 0, 0, 0), 0, vs)]);
    assert_eq!(at(&draw_with(&ram, TEXTURES_ON, &mem), 20, 20).color, GREEN);
}

#[test]
fn given_a_transparent_texel_when_it_covers_a_farther_polygon_then_the_farther_one_shows_and_no_depth_is_written()
 {
    // The near quad's texel 0 is transparent, so it neither draws nor hides the red one behind it.
    let mem = TexMem::new().palette(0, &[colour(1)]);
    let far = quad(16, 16, 24, 24, 100, RED);
    let ram = textured(&[
        (
            opaque(1),
            param(0, 0, COLORS256, COLOR0_CLEAR),
            0,
            tquad((16, 16, 24, 24), -100, (0, 0, 8, 8)),
        ),
        (opaque(2), 0, 0, far),
    ]);
    // The far quad is listed second but the near one is hidden, so red shows through.
    assert_eq!(at(&draw_with(&ram, TEXTURES_ON, &mem), 20, 20).color, RED);
}

#[test]
fn given_a_translucent_polygon_when_its_texel_is_opaque_then_the_pixel_has_the_polygon_alpha() {
    // Modulation multiplies the texel alpha (31) by the polygon alpha (15): 15 again.
    let mem = TexMem::new().image(0, &[1]).palette(0, &[0, WHITE]);
    let attr = 15 << 16 | FRONT_BACK;
    let ram = textured(&[(
        attr,
        param(0, 0, COLORS256, REPEAT_S | REPEAT_T),
        0,
        unit_quad(),
    )]);
    assert_eq!(at(&draw_with(&ram, TEXTURES_ON, &mem), 20, 20).alpha, 15);
}

/// A quad over pixels 16..32 of line 16 whose S runs 0 to 16 texels, with W 3.0 on its left edge
/// and 6.0 on its right (the vertex order of [`tquad`]: two left, then two right).
fn perspective_quad() -> Vec<Vertex> {
    let at = |px: i32, row: i32, k: i32, s: i16| Vertex {
        s: s * 16,
        ..vertex_k(px, row, 0, WHITE, k)
    };
    vec![
        at(16, 16, 1, 0),
        at(16, 17, 1, 0),
        at(32, 17, 2, 16),
        at(32, 16, 2, 16),
    ]
}

#[test]
fn given_a_polygon_with_perspective_when_s_is_interpolated_then_it_follows_1_over_w() {
    // At the middle pixel (t = 1/2 of the screen span) the perspective-correct S is
    // (16 / 2 x 1/2) / (1/2 + 1/2 x 1/2) = 4 / 0.75 = 5.33 texels (linear would be 8): column 5.
    let bytes: Vec<u8> = (0..16).collect();
    let mem = TexMem::new().image(0, &bytes).palette(0, &palette_of(16));
    let ram = textured(&[(opaque(1), param(0, 1, COLORS256, 0), 0, perspective_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    assert_eq!(at(&lines, 24, 16).color, colour(5));
}

#[test]
fn given_a_wire_frame_when_its_top_edge_is_textured_then_each_pixel_reads_its_own_texel() {
    // The top edge of the unit quad runs over texture row 0, one texel to a pixel: pixel k of the
    // edge shows index k.
    let bytes: Vec<u8> = (0..64).collect();
    let mem = TexMem::new().image(0, &bytes).palette(0, &palette_of(64));
    let ram = textured(&[(FRONT_BACK, param(0, 0, COLORS256, 0), 0, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    let got: Vec<u16> = (16..24).map(|x| at(&lines, x, 16).color).collect();
    assert_eq!(got, (0..8).map(colour).collect::<Vec<_>>());
}
