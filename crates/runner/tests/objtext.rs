// SPDX-License-Identifier: Apache-2.0
//! Plain tile objects on both engines, observed through our own `objtext` test ROM
//! (`testroms/objtext/`). The ROM fills the first 8 KiB of each OBJ VRAM with a computed pattern
//! (layout at the top of `arm9.s`) and lists ten objects for engine A (1D mapping, 64-byte
//! boundary, upper screen) and eight for engine B (2D mapping, lower screen): several sizes, 16 and
//! 256 colours, every flip, a negative X, an X at the right edge, a Y that wraps and a Y at the
//! bottom.
//!
//! The expected pictures are worked out here, pixel by pixel, from `docs/hw/obj.md` ("Where an
//! object is on the screen", "A plain tile object", "Priority") and the data the ROM's source
//! lists. They are never read back from the emulator: the code below shares nothing with
//! `ninge-core`.

use ninge_core::debug::EngineId;
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run before the screens are inspected; the ROM finishes its setup inside the first.
const FRAMES: u64 = 3;

fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("objtext")).expect("objtext ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("objtext ROM has a valid header");
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

/// The 15-bit colour as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// The backdrops: engine A green (0x03E0), engine B blue (0x7C00) (arm9.s, "Palettes").
const BACKDROP_A: u32 = 0x0000_FF00;
const BACKDROP_B: u32 = 0x0000_00FF;

/// Byte `offset` of the tile data in an OBJ VRAM (arm9.s, "Tile data"): 256 units of 32 bytes,
/// nothing after them.
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
    y: u16,
    /// Signed.
    x: i32,
    tile: usize,
    eight_bpp: bool,
    /// 0 square, 1 wide, 2 tall.
    shape: usize,
    size: usize,
    h_flip: bool,
    v_flip: bool,
    palette: usize,
}

const fn obj(y: u16, x: i32, tile: usize, shape: usize, size: usize, palette: usize) -> Obj {
    Obj {
        y,
        x,
        tile,
        eight_bpp: false,
        shape,
        size,
        h_flip: false,
        v_flip: false,
        palette,
    }
}

impl Obj {
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
            ..self
        }
    }

    /// Width and height from the shape and size (`obj.md`, "attr1").
    fn dimensions(&self) -> (usize, usize) {
        const TABLE: [[(usize, usize); 4]; 3] = [
            [(8, 8), (16, 16), (32, 32), (64, 64)],
            [(16, 8), (32, 8), (32, 16), (64, 32)],
            [(8, 16), (8, 32), (16, 32), (32, 64)],
        ];
        TABLE[self.shape][self.size]
    }
}

/// Engine A's entries, in the order of the ROM's `oam_a`.
fn objects_a() -> Vec<Obj> {
    vec![
        obj(8, 8, 1, 0, 0, 0),
        obj(8, 24, 4, 0, 1, 3).flips(true, false),
        obj(8, 48, 8, 1, 2, 0).colors256().flips(false, true),
        obj(20, 100, 16, 0, 3, 5),
        obj(8, 180, 3, 2, 1, 9).flips(true, true),
        obj(100, 10, 30, 1, 3, 0).colors256().flips(true, true),
        obj(150, -16, 40, 0, 2, 7),
        obj(90, 248, 12, 0, 1, 1),
        obj(240, 150, 20, 0, 2, 2),
        obj(100, 200, 50, 2, 2, 0).colors256(),
    ]
}

/// Engine B's entries, in the order of the ROM's `oam_b`.
fn objects_b() -> Vec<Obj> {
    vec![
        obj(10, 10, 0x22, 0, 1, 0),
        obj(10, 40, 0x40, 0, 2, 0).colors256(),
        obj(10, 90, 0x61, 0, 1, 0).colors256(),
        obj(40, 120, 0x80, 1, 3, 4).flips(true, true),
        obj(10, 252, 0x10, 2, 0, 6),
        obj(100, -4, 0x20, 1, 1, 0).colors256(),
        obj(188, 100, 0x30, 0, 1, 12),
        obj(0, 120, 0x05, 0, 1, 15),
    ]
}

// ------------------------------------------------------------------ the note's rules

/// Which mapping a scene uses: `Some(boundary)` for 1D with that many bytes, `None` for 2D.
type Mapping = Option<usize>;

/// The palette value of pixel (`column`, `row`) of the object's picture, after the flips.
fn tile_value(o: &Obj, mapping: Mapping, column: usize, row: usize) -> usize {
    let (width, height) = o.dimensions();
    let column = if o.h_flip { width - 1 - column } else { column };
    let row = if o.v_flip { height - 1 - row } else { row };
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

/// The palette index of screen pixel (`x`, `y`) given by one object, or `None`.
fn object_index(o: &Obj, mapping: Mapping, x: usize, y: usize) -> Option<usize> {
    let (width, height) = o.dimensions();
    let row = (y + 256 - usize::from(o.y)) % 256;
    let column = x as i32 - o.x;
    if row >= height || column < 0 || column >= width as i32 {
        return None;
    }
    let value = tile_value(o, mapping, column as usize, row);
    match (value, o.eight_bpp) {
        (0, _) => None,
        (value, true) => Some(value),
        (value, false) => Some(o.palette * 16 + value),
    }
}

/// The expected pixel: the first object in the list with a pixel there (all have priority 0).
fn expected_pixel(objects: &[Obj], mapping: Mapping, backdrop: u32, x: usize, y: usize) -> u32 {
    objects
        .iter()
        .find_map(|o| object_index(o, mapping, x, y))
        .map_or(backdrop, |index| expand(palette(index)))
}

/// The pixels of `screen` that differ from the note's picture: (x, y, expected, got).
fn picture_mismatches(
    machine: &Machine,
    screen: Screen,
    objects: &[Obj],
    mapping: Mapping,
    backdrop: u32,
) -> Vec<(usize, usize, u32, u32)> {
    let frame = machine.frame(screen);
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| {
            (
                x,
                y,
                expected_pixel(objects, mapping, backdrop, x, y),
                frame[y * SCREEN_WIDTH + x],
            )
        })
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

/// How many pixels of the screen are not the backdrop.
fn objects_pixels(machine: &Machine, screen: Screen, backdrop: u32) -> usize {
    machine
        .frame(screen)
        .iter()
        .filter(|&&p| p != backdrop)
        .count()
}

// ------------------------------------------------------------------ the tests

#[test]
fn given_objtext_rom_when_3_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(&machine, Screen::Top, &objects_a(), Some(64), BACKDROP_A);
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_objtext_rom_when_3_frames_run_then_every_pixel_of_the_lower_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(&machine, Screen::Bottom, &objects_b(), None, BACKDROP_B);
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_objtext_rom_when_3_frames_run_then_the_pictures_are_not_blank() {
    // Guards the two tests above against scenes that went blank: the model's own count of
    // non-backdrop pixels, which is the same number the screens must show.
    let machine = boot_frames();
    let model = |objects: &[Obj], mapping: Mapping, backdrop: u32| {
        (0..SCREEN_HEIGHT)
            .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
            .filter(|&(x, y)| expected_pixel(objects, mapping, backdrop, x, y) != backdrop)
            .count()
    };
    let (a, b) = (
        model(&objects_a(), Some(64), BACKDROP_A),
        model(&objects_b(), None, BACKDROP_B),
    );
    assert_eq!(
        (
            a > 1500,
            b > 1500,
            objects_pixels(&machine, Screen::Top, BACKDROP_A) == a,
            objects_pixels(&machine, Screen::Bottom, BACKDROP_B) == b
        ),
        (true, true, true, true)
    );
}

#[test]
fn given_objtext_rom_when_3_frames_run_then_oam_holds_the_listed_entries_and_the_rest_is_disabled()
{
    // The first listed entry of each engine, decoded, and the first unlisted one.
    let machine = boot_frames();
    let view = machine.inspect();
    let (a0, a9, a10) = (
        view.obj(EngineId::A, 0),
        view.obj(EngineId::A, 9),
        view.obj(EngineId::A, 10),
    );
    let (b6, b8) = (view.obj(EngineId::B, 6), view.obj(EngineId::B, 8));
    assert_eq!(
        (
            (a0.y, a0.x, a0.tile, a0.dimensions()),
            (a9.y, a9.x, a9.eight_bpp, a9.dimensions()),
            a10.disabled,
            (b6.y, b6.x, b6.palette),
            b8.disabled
        ),
        (
            (8, 8, 1, Some((8, 8))),
            (100, 200, true, Some((16, 32))),
            true,
            (188, 100, 12),
            true
        )
    );
}

// TODO(hw-unverified): the golden below also pins the model's choices where GBATEK is silent
// (`obj.md`, "Unverified"): the vertical wrap, X as a signed 9-bit number, reads past the OBJ window.
#[test]
fn given_objtext_rom_when_run_for_3_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that these frames are the right ones: the hash pins
    // them.
    assert_frame_hash("objtext", 3);
}

#[test]
fn given_objtext_rom_when_run_twice_for_3_frames_then_every_frame_hash_matches() {
    assert_deterministic("objtext", 3);
}

#[test]
fn given_objtext_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("objtext", 2, 2);
}
