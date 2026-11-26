// SPDX-License-Identifier: Apache-2.0
//! The 4x4 compressed texture format through the rasterizer, worked out by hand from
//! `docs/hw/textures.md` ("4x4 compressed texels"): the block layout, the four palette modes, the
//! palette halfwords in slot 1 for data in slot 0 and slot 2, PLTT_BASE plus the block's palette
//! offset, and what a texture in slot 1 or an unmapped slot 1 reads.
//!
//! Every block of the textures below has rows of `0xE4`: texels 0, 1, 2, 3 from left to right, so a
//! block shows its four palette colours (or transparency) side by side.

use super::texture::*;
use super::*;

/// TEXIMAGE_PARAM format code of 4x4 compressed.
const COMPRESSED: u32 = 5;
/// One row of a block: texels 0, 1, 2, 3 from the left (bits 0-1 first).
const ROW: u8 = 0xE4;
/// Slot 1 starts here in the test's texture memory (image offsets are VRAM offsets).
const SLOT1: usize = 0x2_0000;

/// A 15-bit colour from 5-bit channels.
fn rgb(r: u16, g: u16, b: u16) -> u16 {
    r | g << 5 | b << 10
}

/// The four colours of the modes test: colours 0 to 3 of the palette.
fn palette4() -> [u16; 4] {
    [
        rgb(10, 20, 30),
        rgb(20, 4, 2),
        rgb(31, 31, 31),
        rgb(1, 2, 3),
    ]
}

/// TEXIMAGE_PARAM with `8 << s` texels in S and `8 << t` in T.
fn param_st(offset: u32, s: u32, t: u32, flags: u32) -> u32 {
    param(offset, 0, COMPRESSED, flags) | s << 20 | t << 23
}

/// The 16-bit halfwords as little-endian bytes.
fn halves(h: &[u16]) -> Vec<u8> {
    h.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Texture memory with an 8x8 texture (four blocks) at image offset 0 whose blocks have the palette
/// modes 0, 1, 2, 3 and palette offset 0, and `palette4()` as palette colours 0 to 3.
fn modes_memory() -> TexMem {
    TexMem::new()
        .image(0, &[ROW; 16])
        .image(SLOT1, &halves(&[0x0000, 0x4000, 0x8000, 0xC000]))
        .palette(0, &palette4())
}

/// The pixels of the 8x8 block at `(16, 16)` that are not `want[y][x]` (`None` is hidden):
/// `(x, y, colour, alpha)` of each.
fn block_diffs_opt(lines: &[Line3d], want: &[[Option<u16>; 8]; 8]) -> Vec<(usize, usize, u16, u8)> {
    (0..64)
        .map(|k| (k % 8, k / 8, at(lines, 16 + k % 8, 16 + k / 8)))
        .filter(|(x, y, p)| match want[*y][*x] {
            Some(c) => (p.color, p.alpha) != (c, 31),
            None => p.alpha != 0,
        })
        .map(|(x, y, p)| (x, y, p.color, p.alpha))
        .collect()
}

/// What texel `k` (0 to 3) of a block of mode `mode` shows over `palette4()`: the colour, `None` if
/// transparent. Worked by hand: mode 1's texel 2 is `(c0 + c1) / 2` per channel, mode 3's texels 2
/// and 3 are `(5 c0 + 3 c1) / 8` and `(3 c0 + 5 c1) / 8`, truncating.
fn mode_texel(mode: usize, k: usize) -> Option<u16> {
    let c = palette4();
    let table: [[Option<u16>; 4]; 4] = [
        [Some(c[0]), Some(c[1]), Some(c[2]), None],
        [Some(c[0]), Some(c[1]), Some(rgb(15, 12, 16)), None],
        [Some(c[0]), Some(c[1]), Some(c[2]), Some(c[3])],
        [
            Some(c[0]),
            Some(c[1]),
            Some(rgb(13, 14, 19)),
            Some(rgb(16, 10, 12)),
        ],
    ];
    table[mode][k]
}

/// The expected 8x8 picture of the modes texture: block `b` (row-major, 2 to a row) has mode `b`.
fn modes_picture() -> [[Option<u16>; 8]; 8] {
    std::array::from_fn(|y| std::array::from_fn(|x| mode_texel(y / 4 * 2 + x / 4, x % 4)))
}

#[test]
fn given_four_blocks_of_modes_0_to_3_when_drawn_then_each_mode_shows_its_colours_and_transparency()
{
    let ram = textured(&[(opaque(1), param_st(0, 0, 0, 0), 0, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &modes_memory());
    assert_eq!(block_diffs_opt(&lines, &modes_picture()), vec![]);
}

#[test]
fn given_the_colour_0_bit_set_when_a_compressed_texture_draws_then_colour_0_still_shows() {
    // Bit 29 applies to the 4-, 16- and 256-colour formats only.
    let ram = textured(&[(opaque(1), param_st(0, 0, 0, COLOR0_CLEAR), 0, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &modes_memory());
    assert_eq!(block_diffs_opt(&lines, &modes_picture()), vec![]);
}

#[test]
fn given_pltt_base_2_when_the_palette_halfwords_say_offset_0_then_the_colours_start_at_byte_32() {
    // PLTT_BASE is in 16-byte units: base 2 is byte 32; the palette at 0 is not read.
    let mem = TexMem::new()
        .image(0, &[ROW; 16])
        .image(SLOT1, &halves(&[0x8000; 4]))
        .palette(0, &[rgb(31, 0, 0); 4])
        .palette(32, &palette4());
    let ram = textured(&[(opaque(1), param_st(0, 0, 0, 0), 2, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    assert_eq!(at(&lines, 18, 18).color, palette4()[2]);
}

/// A 16x8 texture (8 blocks, 4 to a row) of texel 0 at image offset 0x40 with mode 2 and palette
/// offset `4 b` (16 bytes a block, in 4-byte units) for block `b`, PLTT_BASE 0x10 (byte 0x100) over
/// colours `colour(k)`, drawn one texel to a pixel; the pixels that do not show `colour(8 b)`.
fn block_order_diffs() -> Vec<(usize, usize, u16)> {
    let infos: Vec<u16> = (0..8).map(|b| 0x8000 | (4 * b)).collect();
    let mem = TexMem::new()
        .image(0x40, &[0; 32])
        .image(SLOT1 + 0x20, &halves(&infos))
        .palette(0x100, &palette_of(64));
    let quad = tquad((16, 16, 32, 24), 0, (0, 0, 16, 8));
    let ram = textured(&[(opaque(1), param_st(0x40, 1, 0, 0), 0x10, quad)]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    (0..128)
        .map(|k| (k % 16, k / 16, at(&lines, 16 + k % 16, 16 + k / 16).color))
        .filter(|(x, y, c)| *c != colour(8 * (y / 4 * 4 + x / 4) as u16))
        .collect()
}

#[test]
fn given_a_16_by_8_texture_when_drawn_then_blocks_run_row_major_with_four_to_a_row() {
    assert_eq!(block_order_diffs(), vec![]);
}

/// Four blocks of an 8x8 texture in slot 2 (image offset 0x40000), mode 2, block `j` at palette
/// offset `4 j` (16 bytes) over colours `colour(120 + k)` from PLTT_BASE 0x40 (byte 0x400); the
/// pixels that do not show `colour(120 + 8 j + texel)` for block `j`.
fn slot_2_diffs() -> Vec<(usize, usize, u16)> {
    let infos: Vec<u16> = (0..4).map(|j| 0x8000 | (4 * j)).collect();
    let mem = TexMem::new()
        .image(0x4_0000, &[ROW; 16])
        // The palette halfwords of slot 2 start 64 KiB into slot 1: offset / 2 + 0x10000.
        .image(SLOT1 + 0x1_0000, &halves(&infos))
        .palette(0x400, &(120..152).map(colour).collect::<Vec<_>>());
    let ram = textured(&[(opaque(1), param_st(0x4_0000, 0, 0, 0), 0x40, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    (0..64)
        .map(|k| (k % 8, k / 8, at(&lines, 16 + k % 8, 16 + k / 8).color))
        .filter(|(x, y, c)| *c != colour(120 + 8 * (y / 4 * 2 + x / 4) as u16 + (x % 4) as u16))
        .collect()
}

#[test]
fn given_a_compressed_texture_in_slot_2_when_drawn_then_its_palette_halfwords_are_in_the_upper_half_of_slot_1()
 {
    assert_eq!(slot_2_diffs(), vec![]);
}

#[test]
fn given_a_compressed_texture_in_slot_1_when_drawn_then_its_palette_halfword_reads_as_0() {
    // No halfword is defined for data in slot 1 (the model reads 0: mode 0, offset 0), so texel 3 is
    // transparent and texels 0 to 2 are colours 0 to 2 of the palette at PLTT_BASE. Slot 1 holds
    // mode-2 halfwords where the formula would look, which must not be used.
    let mem = TexMem::new()
        .image(SLOT1 + 0x4000, &[ROW; 16])
        .image(SLOT1 + 0x1_2000, &halves(&[0x8000; 4]))
        .palette(0, &palette4());
    let ram = textured(&[(opaque(1), param_st(0x2_4000, 0, 0, 0), 0, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &mem);
    let want = std::array::from_fn(|_| std::array::from_fn(|x| mode_texel(0, x % 4)));
    assert_eq!(block_diffs_opt(&lines, &want), vec![]);
}

#[test]
fn given_an_unmapped_slot_1_when_a_slot_0_texture_draws_then_every_block_reads_mode_0_offset_0() {
    let ram = textured(&[(opaque(1), param_st(0, 0, 0, 0), 0, unit_quad())]);
    let lines = draw_with(&ram, TEXTURES_ON, &modes_memory().without_slot(1));
    let want = std::array::from_fn(|_| std::array::from_fn(|x| mode_texel(0, x % 4)));
    assert_eq!(block_diffs_opt(&lines, &want), vec![]);
}
