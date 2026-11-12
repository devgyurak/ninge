// SPDX-License-Identifier: Apache-2.0
//! Object mosaic on both engines, observed through our own `objmosaic` test ROM
//! (`testroms/objmosaic/`). The ROM fills the first 8 KiB of each OBJ VRAM with a computed pattern
//! (read as tiles or, for a bitmap object, as halfwords) and lists eight objects for engine A (1D
//! tile mapping, upper screen, MOSAIC blocks of 5 by 3, the BG fields set too) and five for engine B
//! (2D mapping, lower screen, blocks of 2 by 4): tile, flipped, affine, double-size affine, 256-
//! colour and bitmap objects with the mosaic bit, one cut by the left edge, one that wraps past
//! line 255, and objects without the bit.
//!
//! The expected pictures are worked out here, pixel by pixel, from `docs/hw/obj.md` ("Mosaic",
//! "A plain tile object", "An affine object", "A bitmap object"): an object with attr0 bit 12 shows,
//! at screen pixel (`x`, `y`), what it shows without mosaic at (`x - x mod h`, `y - y mod v`). They
//! are never read back from the emulator: the code below shares nothing with `ninge-core`.

use ninge_core::debug::EngineId;
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const FRAMES: u64 = 3;

/// MOSAIC OBJ block sizes (width, height): engine A 5 by 3, engine B 2 by 4 (arm9.s, "Setup").
type Blocks = (usize, usize);
const BLOCKS_A: Blocks = (5, 3);
const BLOCKS_B: Blocks = (2, 4);

fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("objmosaic")).expect("objmosaic ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("objmosaic ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame());
    machine
}

// ------------------------------------------------------------------ the data of the ROM

/// The OBJ palette entry `index` as a 15-bit colour (arm9.s, "Palettes").
fn palette(index: usize) -> u16 {
    let red = index & 31;
    let green = (3 * index) & 31;
    let blue = (5 * index) & 31;
    (red | green << 5 | blue << 10) as u16
}

fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

const BACKDROP_A: u32 = 0x0000_FF00;
const BACKDROP_B: u32 = 0x0000_00FF;

/// Byte `offset` of the tile data in an OBJ VRAM (arm9.s, "Tile data").
fn vram_byte(offset: usize) -> u8 {
    let (unit, within) = (offset / 32, offset % 32);
    let (row, byte) = (within / 4, within % 4);
    let pixel = |c: usize| ((unit * 5 + 3 * row + c) & 15) as u8;
    match unit < 256 {
        true => pixel(2 * byte) | pixel(2 * byte + 1) << 4,
        false => 0,
    }
}

/// One object as the ROM lists it.
#[derive(Clone, Copy)]
struct Obj {
    y: usize,
    /// Signed.
    x: i32,
    tile: usize,
    eight_bpp: bool,
    shape: usize,
    size: usize,
    palette: usize,
    h_flip: bool,
    v_flip: bool,
    /// The parameter group, or `None` for a plain object.
    group: Option<usize>,
    double: bool,
    mosaic: bool,
    bitmap: bool,
}

const fn affine(y: usize, x: i32, tile: usize, shape: usize, size: usize, group: usize) -> Obj {
    Obj {
        y,
        x,
        tile,
        eight_bpp: false,
        shape,
        size,
        palette: 0,
        h_flip: false,
        v_flip: false,
        group: Some(group),
        double: false,
        mosaic: false,
        bitmap: false,
    }
}

impl Obj {
    const fn mosaic(self) -> Self {
        Self {
            mosaic: true,
            ..self
        }
    }
    const fn bitmap(self) -> Self {
        Self {
            bitmap: true,
            ..self
        }
    }
    const fn double(self) -> Self {
        Self {
            double: true,
            ..self
        }
    }
    const fn colors256(self) -> Self {
        Self {
            eight_bpp: true,
            ..self
        }
    }
    const fn flips(self, h_flip: bool, v_flip: bool) -> Self {
        Self {
            h_flip,
            v_flip,
            group: None,
            ..self
        }
    }
    const fn palette(self, palette: usize) -> Self {
        Self { palette, ..self }
    }

    fn dimensions(&self) -> (usize, usize) {
        const TABLE: [[(usize, usize); 4]; 3] = [
            [(8, 8), (16, 16), (32, 32), (64, 64)],
            [(16, 8), (32, 8), (32, 16), (64, 32)],
            [(8, 16), (8, 32), (16, 32), (32, 64)],
        ];
        TABLE[self.shape][self.size]
    }
}

/// A plain object (no group).
const fn plain(y: usize, x: i32, tile: usize, shape: usize, size: usize) -> Obj {
    Obj {
        group: None,
        ..affine(y, x, tile, shape, size, 0)
    }
}

fn objects_a() -> Vec<Obj> {
    vec![
        plain(20, 20, 8, 0, 2).palette(3).mosaic(),
        plain(20, 70, 8, 0, 2).palette(3),
        plain(70, 20, 40, 0, 1)
            .palette(5)
            .flips(true, false)
            .mosaic(),
        affine(70, 70, 8, 0, 2, 2).palette(3).mosaic(),
        affine(110, 20, 2, 0, 1, 3).double().palette(1).mosaic(),
        plain(150, 20, 64, 1, 3).colors256().mosaic(),
        plain(110, 120, 0, 0, 2).bitmap().mosaic(),
        plain(150, -10, 12, 0, 1).palette(2).mosaic(),
    ]
}

fn objects_b() -> Vec<Obj> {
    vec![
        plain(10, 10, 0x40, 0, 2).palette(2).mosaic(),
        plain(10, 60, 0x22, 0, 1).flips(false, true).mosaic(),
        affine(60, 10, 0x60, 1, 2, 1).double().palette(4).mosaic(),
        plain(60, 120, 0x22, 0, 1),
        plain(250, 150, 0x30, 0, 1).palette(12).mosaic(),
    ]
}

/// Parameter group `group`: PA, PB, PC, PD (arm9.s, "Parameter groups").
fn parameters(group: usize) -> [i32; 4] {
    match group {
        1 => [0, 256, -256, 0],
        2 => [222, 128, -128, 222],
        3 => [128, 0, 0, 128],
        _ => [0, 0, 0, 0],
    }
}

// ------------------------------------------------------------------ the note's rules

/// Which mapping a scene uses: `Some(boundary)` for 1D with that many bytes, `None` for 2D.
type Mapping = Option<usize>;

/// The palette value of pixel (`column`, `row`) of the object's picture.
fn tile_value(o: &Obj, mapping: Mapping, column: usize, row: usize) -> usize {
    let (width, _) = o.dimensions();
    let (tile_column, tile_row, x, y) = (column / 8, row / 8, column % 8, row % 8);
    let tile_bytes = if o.eight_bpp { 64 } else { 32 };
    let tile_at = match mapping {
        Some(boundary) => o.tile * boundary + (tile_column + tile_row * (width / 8)) * tile_bytes,
        None => {
            let (first, step) = if o.eight_bpp {
                (o.tile & !1, 2)
            } else {
                (o.tile, 1)
            };
            (first + tile_column * step + tile_row * 32) * 32
        }
    };
    if o.eight_bpp {
        usize::from(vram_byte(tile_at + y * 8 + x))
    } else {
        let byte = vram_byte(tile_at + y * 4 + x / 2);
        usize::from(if x % 2 == 0 { byte & 0xF } else { byte >> 4 })
    }
}

/// The 15-bit colour of picture pixel (`column`, `row`) of the object: a bitmap halfword with bit
/// 15 set, or a tile pixel through the palette; `None` where transparent.
fn picture_color(o: &Obj, mapping: Mapping, column: usize, row: usize) -> Option<u16> {
    if o.bitmap {
        // 2D mapping, source bitmap 128 wide (DISPCNT bits 5 and 6 are clear).
        let first = (o.tile & 0xF) * 0x10 + (o.tile & !0xF) * 0x80;
        let at = first + row * 128 * 2 + column * 2;
        let half = u16::from(vram_byte(at)) | u16::from(vram_byte(at + 1)) << 8;
        return (half & 0x8000 != 0).then_some(half & 0x7FFF);
    }
    match (tile_value(o, mapping, column, row), o.eight_bpp) {
        (0, _) => None,
        (value, true) => Some(palette(value)),
        (value, false) => Some(palette(o.palette * 16 + value)),
    }
}

/// The 15-bit colour the object shows at screen pixel (`x`, `y`) without mosaic, or `None`.
fn object_color_at(o: &Obj, mapping: Mapping, x: i32, y: usize) -> Option<u16> {
    let (width, height) = o.dimensions();
    let (box_width, box_height) = if o.double {
        (2 * width, 2 * height)
    } else {
        (width, height)
    };
    let row = (y + 256 - o.y) % 256;
    let column = usize::try_from(x - o.x).ok()?;
    if row >= box_height || column >= box_width {
        return None;
    }
    let (u, v) = match o.group {
        Some(group) => {
            let [pa, pb, pc, pd] = parameters(group);
            let dx = column as i32 - (box_width / 2) as i32;
            let dy = row as i32 - (box_height / 2) as i32;
            (
                ((pa * dx + pb * dy) >> 8) + (width / 2) as i32,
                ((pc * dx + pd * dy) >> 8) + (height / 2) as i32,
            )
        }
        None => (
            if o.h_flip {
                width as i32 - 1 - column as i32
            } else {
                column as i32
            },
            if o.v_flip {
                height as i32 - 1 - row as i32
            } else {
                row as i32
            },
        ),
    };
    if !(0..width as i32).contains(&u) || !(0..height as i32).contains(&v) {
        return None;
    }
    picture_color(o, mapping, u as usize, v as usize)
}

/// The 15-bit colour of screen pixel (`x`, `y`) given by one object: with mosaic, what the object
/// shows at the first pixel of the block (blocks are counted from the top left of the screen).
fn object_color(o: &Obj, mapping: Mapping, blocks: Blocks, x: usize, y: usize) -> Option<u16> {
    let (block_width, block_height) = if o.mosaic { blocks } else { (1, 1) };
    object_color_at(
        o,
        mapping,
        (x - x % block_width) as i32,
        y - y % block_height,
    )
}

/// The expected pixel: the first object in the list with a pixel there (all have priority 0).
fn expected_pixel(
    objects: &[Obj],
    mapping: Mapping,
    blocks: Blocks,
    backdrop: u32,
    x: usize,
    y: usize,
) -> u32 {
    objects
        .iter()
        .find_map(|o| object_color(o, mapping, blocks, x, y))
        .map_or(backdrop, expand)
}

fn picture_mismatches(
    machine: &Machine,
    screen: Screen,
    objects: &[Obj],
    mapping: Mapping,
    blocks: Blocks,
    backdrop: u32,
) -> Vec<(usize, usize, u32, u32)> {
    let frame = machine.frame(screen);
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| {
            (
                x,
                y,
                expected_pixel(objects, mapping, blocks, backdrop, x, y),
                frame[y * SCREEN_WIDTH + x],
            )
        })
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

// ------------------------------------------------------------------ the tests

#[test]
fn given_objmosaic_rom_when_3_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(
        &machine,
        Screen::Top,
        &objects_a(),
        Some(32),
        BLOCKS_A,
        BACKDROP_A,
    );
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_objmosaic_rom_when_3_frames_run_then_every_pixel_of_the_lower_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(
        &machine,
        Screen::Bottom,
        &objects_b(),
        None,
        BLOCKS_B,
        BACKDROP_B,
    );
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_objmosaic_rom_when_3_frames_run_then_the_pictures_are_not_blank_and_the_mosaic_changes_them()
 {
    // The model's own count of non-backdrop pixels must be what the screens show, and the same
    // objects without the mosaic bit must give a different picture (a vacuous mosaic would not).
    let machine = boot_frames();
    let count = |objects: &[Obj], mapping: Mapping, blocks: Blocks, backdrop: u32| {
        (0..SCREEN_HEIGHT)
            .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
            .filter(|&(x, y)| expected_pixel(objects, mapping, blocks, backdrop, x, y) != backdrop)
            .count()
    };
    let shown = |screen: Screen, backdrop: u32| {
        machine
            .frame(screen)
            .iter()
            .filter(|&&p| p != backdrop)
            .count()
    };
    let without_mosaic = |objects: Vec<Obj>| -> Vec<Obj> {
        objects
            .into_iter()
            .map(|o| Obj { mosaic: false, ..o })
            .collect()
    };
    let (a, b) = (
        count(&objects_a(), Some(32), BLOCKS_A, BACKDROP_A),
        count(&objects_b(), None, BLOCKS_B, BACKDROP_B),
    );
    let (plain_a, plain_b) = (
        count(&without_mosaic(objects_a()), Some(32), BLOCKS_A, BACKDROP_A),
        count(&without_mosaic(objects_b()), None, BLOCKS_B, BACKDROP_B),
    );
    assert_eq!(
        (
            a > 2000,
            b > 1000,
            shown(Screen::Top, BACKDROP_A) == a,
            shown(Screen::Bottom, BACKDROP_B) == b,
            plain_a != a,
            plain_b != b
        ),
        (true, true, true, true, true, true)
    );
}

#[test]
fn given_objmosaic_rom_when_3_frames_run_then_oam_holds_mosaic_objects_and_the_registers_are_the_roms()
 {
    let machine = boot_frames();
    let view = machine.inspect();
    let (a0, a1, a6) = (
        view.obj(EngineId::A, 0),
        view.obj(EngineId::A, 1),
        view.obj(EngineId::A, 6),
    );
    let (mosaic_a, mosaic_b) = (
        view.gpu2d().mosaic(EngineId::A),
        view.gpu2d().mosaic(EngineId::B),
    );
    assert_eq!(
        (
            (a0.mosaic, a1.mosaic, a6.mosaic, a6.mode),
            (mosaic_a.obj_h, mosaic_a.obj_v, mosaic_a.bg_h, mosaic_a.bg_v),
            (mosaic_b.obj_h, mosaic_b.obj_v)
        ),
        ((true, false, true, 3), (5, 3, 6, 4), (2, 4))
    );
}

#[test]
fn given_objmosaic_rom_when_run_for_3_frames_then_both_screens_match_golden() {
    assert_frame_hash("objmosaic", 3);
}

#[test]
fn given_objmosaic_rom_when_run_twice_for_3_frames_then_every_frame_hash_matches() {
    assert_deterministic("objmosaic", 3);
}

#[test]
fn given_objmosaic_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run()
{
    assert_savestate_round_trip("objmosaic", 2, 2);
}
