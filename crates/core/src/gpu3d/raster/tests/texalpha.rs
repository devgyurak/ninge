// SPDX-License-Identifier: Apache-2.0
//! The alpha texture formats (A3I5, A5I3, direct) and decal shading through the rasterizer, worked
//! out by hand from `docs/hw/textures.md` ("Texture alpha, translucency and the buffers", "Blending
//! the texel with the vertex colour").
//!
//! As in `texture.rs`, a quad 8 pixels wide with S running 0 to 8 reads one texel per pixel, and a
//! white vertex colour makes modulation the identity on the colour.

use super::texture::*;
use super::*;
use crate::gpu3d::polygon::Ram;
use crate::gpu3d::raster::texture::{Texel, decal};

/// TEXIMAGE_PARAM format codes.
const A3I5: u32 = 1;
const A5I3: u32 = 6;
const DIRECT: u32 = 7;
/// DISP3DCNT bit 2 (alpha test) and bit 3 (blending).
const ALPHA_TEST: u16 = 1 << 2;
const BLENDING: u16 = 1 << 3;
/// POLYGON_ATTR polygon mode 1, decal.
const DECAL: u32 = 1 << 4;

/// All lines of `ram` over texture memory `mem` with registers `regs`.
fn draw_regs(ram: &Ram, disp3dcnt: u16, mem: &TexMem, regs: &RenderRegs) -> Vec<Line3d> {
    let frame = Frame::new(Scene { ram, swap_bits: 0 }, regs, disp3dcnt);
    let mut lines = vec![[Pixel3d::default(); SCREEN_WIDTH]; crate::SCREEN_HEIGHT];
    frame.draw(&SequentialExecutor, &mem.view(), &mut lines);
    lines
}

/// The alpha of the pixel in column 20 on each of the 8 rows of the unit quad (16..24).
fn row_alphas(lines: &[Line3d]) -> Vec<u8> {
    (16..24).map(|y| at(lines, 20, y).alpha).collect()
}

/// A texture of 8x8 one-byte texels where row `y` holds `row(y)`, at image offset 0.
fn rows(row: impl Fn(u8) -> u8) -> Vec<u8> {
    (0..64).map(|i| row(i / 8)).collect()
}

/// A texture of 64 one-byte texels that all hold `value`.
fn filled(value: u8) -> Vec<u8> {
    vec![value; 64]
}

#[test]
fn given_an_a3i5_texture_when_each_row_has_a_3_bit_alpha_then_it_widens_to_five_bits() {
    // alpha a becomes a x 4 + a / 2: 0, 4, 9, 13, 18, 22, 27, 31 (worked by hand); alpha 0 hides.
    let mem = TexMem::new()
        .image(0, &rows(|y| 1 | y << 5))
        .palette(0, &palette_of(32));
    let ram = textured(&[(opaque(1), param(0, 0, A3I5, 0), 0, unit_quad())]);
    let alphas = row_alphas(&draw_with(&ram, TEXTURES_ON, &mem));
    assert_eq!(alphas, vec![0, 4, 9, 13, 18, 22, 27, 31]);
}

#[test]
fn given_an_a3i5_texture_when_drawn_then_the_low_5_bits_index_a_32_colour_palette() {
    // Texel x of the row has index 4x + 1, so the indices reach past 7, with alpha 7 (solid).
    let bytes: Vec<u8> = (0..64u8).map(|i| ((i % 8) * 4 + 1) | 7 << 5).collect();
    let mem = TexMem::new().image(0, &bytes).palette(0, &palette_of(32));
    let ram = textured(&[(opaque(1), param(0, 0, A3I5, 0), 0, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    assert_eq!(block_diffs(&lines, &grid(|x, _| colour(x * 4 + 1))), vec![]);
}

#[test]
fn given_an_a5i3_texture_when_drawn_then_the_alpha_is_the_top_5_bits_and_the_index_the_low_3() {
    // Row y has alpha 5 + y and texel x index x; PLTT_BASE 2 is byte 32 of the palette space.
    let bytes: Vec<u8> = (0..64u8).map(|i| (i % 8) | (5 + i / 8) << 3).collect();
    let mem = TexMem::new().image(0, &bytes).palette(32, &palette_of(8));
    let ram = textured(&[(opaque(1), param(0, 0, A5I3, 0), 2, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    assert_eq!(row_alphas(&lines), vec![5, 6, 7, 8, 9, 10, 11, 12]);
    assert_eq!(at(&lines, 23, 20).color, colour(7));
}

/// The texels of an 8x8 direct texture (texel `i` is `colour(i)`, solid unless `i` is a multiple of
/// 3) whose pixel is not the colour and alpha it should be: `(i, colour, alpha)`.
fn direct_texture_diffs() -> Vec<(usize, u16, u8)> {
    let solid = |i: usize| !i.is_multiple_of(3);
    let direct: Vec<u16> = (0..64)
        .map(|i| colour(i as u16) | [0, 0x8000][usize::from(solid(i))])
        .collect();
    let bytes: Vec<u8> = direct.iter().flat_map(|h| h.to_le_bytes()).collect();
    let mem = TexMem::new().image(0, &bytes);
    let ram = textured(&[(opaque(1), param(0, 0, DIRECT, 0), 0, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    (0..64)
        .map(|i| (i, at(&lines, 16 + i % 8, 16 + i / 8)))
        .filter(|(i, p)| {
            solid(*i) && (p.color, p.alpha) != (colour(*i as u16), 31) || !solid(*i) && p.alpha != 0
        })
        .map(|(i, p)| (i, p.color, p.alpha))
        .collect()
}

#[test]
fn given_a_direct_texture_when_drawn_then_bit_15_is_the_alpha_and_the_low_15_bits_the_colour() {
    assert_eq!(direct_texture_diffs(), vec![]);
}

#[test]
fn given_the_colour_0_bit_set_when_a_translucent_format_has_index_0_then_the_alpha_decides_not_the_bit()
 {
    // Index 0 with alpha 31 in A5I3: bit 29 only applies to the 4-, 16- and 256-colour formats.
    let mem = TexMem::new()
        .image(0, &filled(31 << 3))
        .palette(0, &palette_of(8));
    let ram = textured(&[(
        opaque(1),
        param(0, 0, A5I3, COLOR0_CLEAR | REPEAT_S | REPEAT_T),
        0,
        unit_quad(),
    )]);
    assert_eq!(at(&draw_with(&ram, TEXTURES_ON, &mem), 20, 20).alpha, 31);
}

#[test]
fn given_a_translucent_polygon_when_its_texel_alpha_is_partial_then_the_two_alphas_multiply() {
    // Texel alpha 16 (6-bit 33) times polygon alpha 15 (6-bit 31): (34 x 32 - 1) / 64 = 16, and
    // dropping the low bit gives 8 (worked by hand).
    let mem = TexMem::new()
        .image(0, &filled(16 << 3))
        .palette(0, &palette_of(8));
    let attr = 15 << 16 | FRONT_BACK;
    let ram = textured(&[(attr, param(0, 0, A5I3, REPEAT_S | REPEAT_T), 0, unit_quad())]);
    assert_eq!(at(&draw_with(&ram, TEXTURES_ON, &mem), 20, 20).alpha, 8);
}

#[test]
fn given_a_wire_frame_when_its_a5i3_texel_has_alpha_16_then_its_lines_draw_at_16_and_the_inside_is_empty()
 {
    // A wire frame has a vertex alpha of 31, so only the texel's alpha makes its lines translucent.
    let mem = TexMem::new()
        .image(0, &filled(16 << 3))
        .palette(0, &palette_of(8));
    let attr = FRONT_BACK;
    let ram = textured(&[(attr, param(0, 0, A5I3, REPEAT_S | REPEAT_T), 0, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    assert_eq!(
        (at(&lines, 20, 16).alpha, at(&lines, 20, 20).alpha),
        (16, 0)
    );
}

#[test]
fn given_the_alpha_test_when_texel_alphas_run_from_5_to_12_then_those_up_to_the_reference_8_are_hidden()
 {
    let bytes = rows(|y| 1 | (5 + y) << 3);
    let mem = TexMem::new().image(0, &bytes).palette(0, &palette_of(8));
    let ram = textured(&[(opaque(1), param(0, 0, A5I3, 0), 0, unit_quad())]);
    let regs = RenderRegs {
        alpha_test_ref: 8,
        ..regs()
    };
    let lines = draw_regs(&ram, TEXTURES_ON | ALPHA_TEST, &mem, &regs);
    assert_eq!(row_alphas(&lines), vec![0, 0, 0, 0, 9, 10, 11, 12]);
}

#[test]
fn given_a_partly_transparent_texture_when_it_is_nearer_than_an_opaque_polygon_then_it_blends_over_it()
 {
    // The A5I3 quad is listed first and nearer, with texel alpha 15; its polygon draws with the
    // translucent ones, after the red quad, and blends with it: (new x 16 + old x 16) / 32 per 6-bit
    // channel, then narrowed. Texel colour(3) is 5-bit (3, 9, 21), 6-bit (7, 19, 43); red is
    // (63, 0, 0): (35, 9, 21) and 5-bit (17, 4, 10).
    let mem = TexMem::new()
        .image(0, &filled(3 | 15 << 3))
        .palette(0, &palette_of(8));
    let near = tquad((16, 16, 24, 24), -500, (0, 0, 8, 8));
    let ram = textured(&[
        (opaque(1), param(0, 0, A5I3, REPEAT_S | REPEAT_T), 0, near),
        (opaque(2), 0, 0, quad(16, 16, 24, 24, 500, RED)),
    ]);
    let p = at(&draw_with(&ram, TEXTURES_ON | BLENDING, &mem), 20, 20);
    assert_eq!(p.color, 17 | 4 << 5 | 10 << 10);
}

#[test]
fn given_an_a5i3_texel_with_alpha_0_when_it_covers_a_farther_polygon_then_that_polygon_shows() {
    let mem = TexMem::new().image(0, &[0]).palette(0, &palette_of(8));
    let near = tquad((16, 16, 24, 24), -500, (0, 0, 8, 8));
    let ram = textured(&[
        (opaque(1), param(0, 0, A5I3, REPEAT_S | REPEAT_T), 0, near),
        (opaque(2), 0, 0, quad(16, 16, 24, 24, 500, GREEN)),
    ]);
    assert_eq!(at(&draw_with(&ram, TEXTURES_ON, &mem), 20, 20).color, GREEN);
}

/// The 6-bit result of decal-blending a texel of 5-bit colour `texel` and alpha `alpha` with a vertex
/// colour whose 5-bit channels are `vertex`, red channel only.
fn decal_red(texel: u16, alpha: u8, vertex: u16) -> u8 {
    let six = |c: u16| crate::gpu3d::raster::texture::expand6(c);
    decal(
        Texel {
            color: texel,
            alpha,
        },
        [six(vertex); 3],
        31,
    )
    .0[0]
}

#[test]
fn given_texel_alphas_when_decalled_then_0_gives_the_vertex_colour_31_the_texel_and_the_rest_a_mix()
{
    // (texel, alpha, vertex, wanted 6-bit red), worked by hand with At6 = 2 x alpha + 1:
    // alpha 0 -> vertex 63; alpha 31 -> texel 63; alpha 16 (At6 33): white texel over black vertex
    // (63 x 33) / 64 = 32; black texel over a white vertex (63 x 30) / 64 = 29.
    let cases: [(u16, u8, u16, u8); 4] = [
        (0, 0, 31, 63),
        (31, 31, 0, 63),
        (31, 16, 0, 32),
        (0, 16, 31, 29),
    ];
    let wrong: Vec<(u16, u8, u16, u8, u8)> = cases
        .iter()
        .map(|&(t, a, v, want)| (t, a, v, decal_red(t, a, v), want))
        .filter(|(.., got, want)| got != want)
        .collect();
    assert_eq!(wrong, vec![]);
}

#[test]
fn given_a_decal_polygon_when_the_texel_alpha_is_partial_then_the_pixel_is_opaque_and_mixed() {
    // A5I3 alpha 16 over a red vertex colour: the polygon's alpha 31 stays, the colour is the mix of
    // the texel colour(2) = (2, 6, 14) -> 6-bit (5, 13, 29) and red (63, 0, 0) at At6 33:
    // ((5 x 33 + 63 x 30) / 64, (13 x 33) / 64, (29 x 33) / 64) = (32, 6, 14), 5-bit (16, 3, 7).
    let mem = TexMem::new()
        .image(0, &filled(2 | 16 << 3))
        .palette(0, &palette_of(8));
    let vs: Vec<Vertex> = unit_quad()
        .into_iter()
        .map(|v| Vertex { color: RED, ..v })
        .collect();
    let ram = textured(&[(
        opaque(1) | DECAL,
        param(0, 0, A5I3, REPEAT_S | REPEAT_T),
        0,
        vs,
    )]);
    let p = at(&draw_with(&ram, TEXTURES_ON | BLENDING, &mem), 20, 20);
    assert_eq!((p.color, p.alpha), (16 | 3 << 5 | 7 << 10, 31));
}

#[test]
fn given_a_decal_polygon_when_a_direct_texel_is_transparent_then_the_vertex_colour_shows_at_the_polygon_alpha()
 {
    let mem = TexMem::new().image(0, &[0, 0]);
    let vs: Vec<Vertex> = unit_quad()
        .into_iter()
        .map(|v| Vertex { color: GREEN, ..v })
        .collect();
    let ram = textured(&[(
        opaque(1) | DECAL,
        param(0, 0, DIRECT, REPEAT_S | REPEAT_T),
        0,
        vs,
    )]);
    let p = at(&draw_with(&ram, TEXTURES_ON, &mem), 20, 20);
    assert_eq!((p.color, p.alpha), (GREEN, 31));
}
