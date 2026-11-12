// SPDX-License-Identifier: Apache-2.0
//! Affine objects on both engines, observed through our own `objaffine` test ROM
//! (`testroms/objaffine/`). The ROM fills the first 8 KiB of each OBJ VRAM with a computed pattern
//! and lists eight objects for each engine (engine A: 1D mapping, upper screen; engine B: 2D
//! mapping, lower screen): rotations, scales and a shear through parameter groups 0-5 and 31,
//! double-size boxes, a box cut by the right edge, a box that wraps past line 255, and a plain
//! object with the disable bit.
//!
//! The expected pictures are worked out here, pixel by pixel, from `docs/hw/obj.md` ("An affine
//! object"): for box pixel (`bx`, `by`), `dx = bx - boxW / 2`, `dy = by - boxH / 2`, `u = ((PA *
//! dx + PB * dy) >> 8) + W / 2`, `v = ((PC * dx + PD * dy) >> 8) + H / 2`, and the pixel shows the
//! picture at (`u`, `v`) when that is inside it. They are never read back from the emulator: the
//! code below shares nothing with `ninge-core`.

use ninge_core::debug::EngineId;
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const FRAMES: u64 = 3;

fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("objaffine")).expect("objaffine ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("objaffine ROM has a valid header");
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
    x: usize,
    tile: usize,
    eight_bpp: bool,
    shape: usize,
    size: usize,
    palette: usize,
    /// The parameter group, or `None` for a plain object.
    group: Option<usize>,
    double: bool,
}

const fn affine(y: usize, x: usize, tile: usize, shape: usize, size: usize, group: usize) -> Obj {
    Obj {
        y,
        x,
        tile,
        eight_bpp: false,
        shape,
        size,
        palette: 0,
        group: Some(group),
        double: false,
    }
}

impl Obj {
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

/// A plain object with the disable bit set: it draws nothing, so it has no group to draw with.
const fn disabled(y: usize, x: usize) -> Obj {
    Obj {
        group: None,
        ..affine(y, x, 1, 0, 0, 0)
    }
}

fn objects_a() -> Vec<Obj> {
    vec![
        affine(20, 20, 8, 0, 2, 2).palette(3),
        affine(20, 80, 8, 0, 2, 2).double().palette(3),
        affine(20, 160, 40, 0, 1, 3).palette(5),
        affine(60, 20, 64, 0, 3, 31).double().colors256(),
        affine(70, 170, 100, 2, 2, 1).palette(9),
        affine(150, 200, 2, 0, 0, 4).double().palette(1),
        disabled(20, 230),
        affine(100, 226, 12, 0, 1, 0).double().palette(2),
    ]
}

fn objects_b() -> Vec<Obj> {
    vec![
        affine(10, 10, 0x20, 1, 2, 1),
        affine(10, 60, 0x40, 0, 1, 5).double().colors256(),
        affine(50, 100, 0x60, 1, 3, 2).double().palette(4),
        affine(100, 10, 0x08, 2, 1, 3).palette(6),
        affine(120, 50, 0x80, 0, 2, 31).palette(7),
        disabled(20, 200),
        affine(150, 230, 0x10, 0, 1, 0).palette(12),
        affine(240, 150, 0x14, 0, 1, 0).double().palette(1),
    ]
}

/// Parameter group `group`: PA, PB, PC, PD (arm9.s, "Parameter groups").
fn parameters(group: usize) -> [i32; 4] {
    match group {
        0 => [256, 0, 0, 256],
        1 => [0, 256, -256, 0],
        2 => [222, 128, -128, 222],
        3 => [128, 0, 0, 128],
        4 => [512, 0, 0, 512],
        5 => [256, 64, 0, 256],
        31 => [181, -181, 181, 181],
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

/// The palette index of screen pixel (`x`, `y`) given by one object, or `None`.
fn object_index(o: &Obj, mapping: Mapping, x: usize, y: usize) -> Option<usize> {
    let group = o.group?;
    let (width, height) = o.dimensions();
    let (box_width, box_height) = if o.double {
        (2 * width, 2 * height)
    } else {
        (width, height)
    };
    let row = (y + 256 - o.y) % 256;
    let column = x.checked_sub(o.x)?;
    if row >= box_height || column >= box_width {
        return None;
    }
    let [pa, pb, pc, pd] = parameters(group);
    let dx = column as i32 - (box_width / 2) as i32;
    let dy = row as i32 - (box_height / 2) as i32;
    let u = ((pa * dx + pb * dy) >> 8) + (width / 2) as i32;
    let v = ((pc * dx + pd * dy) >> 8) + (height / 2) as i32;
    if !(0..width as i32).contains(&u) || !(0..height as i32).contains(&v) {
        return None;
    }
    match (tile_value(o, mapping, u as usize, v as usize), o.eight_bpp) {
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

// ------------------------------------------------------------------ the tests

#[test]
fn given_objaffine_rom_when_3_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(&machine, Screen::Top, &objects_a(), Some(32), BACKDROP_A);
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_objaffine_rom_when_3_frames_run_then_every_pixel_of_the_lower_screen_is_the_pictures_the_note_gives()
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
fn given_objaffine_rom_when_3_frames_run_then_the_pictures_are_not_blank() {
    // Guards the two tests above against scenes that went blank: the model's own count of
    // non-backdrop pixels must be what the screens show.
    let machine = boot_frames();
    let count = |objects: &[Obj], mapping: Mapping, backdrop: u32| {
        (0..SCREEN_HEIGHT)
            .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
            .filter(|&(x, y)| expected_pixel(objects, mapping, backdrop, x, y) != backdrop)
            .count()
    };
    let shown = |screen: Screen, backdrop: u32| {
        machine
            .frame(screen)
            .iter()
            .filter(|&&p| p != backdrop)
            .count()
    };
    let (a, b) = (
        count(&objects_a(), Some(32), BACKDROP_A),
        count(&objects_b(), None, BACKDROP_B),
    );
    assert_eq!(
        (
            a > 3000,
            b > 2000,
            shown(Screen::Top, BACKDROP_A) == a,
            shown(Screen::Bottom, BACKDROP_B) == b
        ),
        (true, true, true, true)
    );
}

#[test]
fn given_objaffine_rom_when_3_frames_run_then_oam_holds_the_listed_entries_and_parameter_groups() {
    let machine = boot_frames();
    let view = machine.inspect();
    let (a1, a3, a6) = (
        view.obj(EngineId::A, 1),
        view.obj(EngineId::A, 3),
        view.obj(EngineId::A, 6),
    );
    let b7 = view.obj(EngineId::B, 7);
    assert_eq!(
        (
            (a1.affine, a1.double_size, a1.group),
            (a3.affine, a3.double_size, a3.group, a3.eight_bpp),
            (a6.affine, a6.disabled),
            (b7.y, b7.double_size, b7.group)
        ),
        (
            (true, true, 2),
            (true, true, 31, true),
            (false, true),
            (240, true, 0)
        )
    );
}

#[test]
fn given_objaffine_rom_when_run_for_3_frames_then_both_screens_match_golden() {
    assert_frame_hash("objaffine", 3);
}

#[test]
fn given_objaffine_rom_when_run_twice_for_3_frames_then_every_frame_hash_matches() {
    assert_deterministic("objaffine", 3);
}

#[test]
fn given_objaffine_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run()
{
    assert_savestate_round_trip("objaffine", 2, 2);
}
