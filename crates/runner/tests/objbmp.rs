// SPDX-License-Identifier: Apache-2.0
//! Bitmap objects on both engines, observed through our own `objbmp` test ROM
//! (`testroms/objbmp/`). The ROM fills the first 64 KiB of each OBJ VRAM with a computed bitmap
//! (one pixel in five has bit 15 clear: transparent) and lists ten objects for engine A, shown in
//! three line bands with three mappings (2D 256 wide, 2D 128 wide, 1D with a 256-byte boundary,
//! the ROM changes DISPCNT after lines 63 and 127), and six for engine B (1D, 128-byte
//! boundary): a plain object in each mapping, flips, an affine object with a quarter turn, a
//! magnified one and one with a rotation in a double-size box, an object clipped at the left edge
//! and one whose attr2 bits 12-15 (the alpha field, not applied yet) are set.
//!
//! The expected pictures are worked out here, pixel by pixel, from `docs/hw/obj.md` ("A bitmap
//! object") and the data the ROM's source lists. They are never read back from the emulator: the
//! code below shares nothing with `ninge-core`.

use ninge_core::debug::EngineId;
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run before the screens are inspected: the first frames hold the setup and the first
/// DISPCNT bands, which settle once the ROM's loop has seen a whole frame.
const FRAMES: u64 = 5;

fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("objbmp")).expect("objbmp ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("objbmp ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame());
    machine
}

// ------------------------------------------------------------------ the data of the ROM

fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// The backdrops: engine A green (0x03E0), engine B blue (0x7C00).
const BACKDROP_A: u32 = 0x0000_FF00;
const BACKDROP_B: u32 = 0x0000_00FF;

/// Halfword `index` of the bitmap data in an OBJ VRAM (arm9.s, "Bitmap data"): 32768 halfwords,
/// nothing after them.
fn vram_half(byte: usize) -> u16 {
    let index = byte / 2;
    match index < 32768 {
        true => {
            let color = ((index * 0x2F1) & 0x7FFF) as u16;
            if index.is_multiple_of(5) {
                color
            } else {
                color | 0x8000
            }
        }
        false => 0,
    }
}

/// How the bitmap objects of a band are mapped (`obj.md`, "A bitmap object").
#[derive(Clone, Copy)]
enum Mapping {
    /// 2D with a source bitmap of this many pixels across.
    TwoD(usize),
    /// 1D with this boundary in bytes.
    OneD(usize),
}

/// One object as the ROM lists it.
#[derive(Clone, Copy)]
struct Obj {
    y: usize,
    /// Signed.
    x: i32,
    tile: usize,
    shape: usize,
    size: usize,
    h_flip: bool,
    v_flip: bool,
    /// The parameter group, or `None` for a plain object.
    group: Option<usize>,
    double: bool,
}

const fn obj(y: usize, x: i32, tile: usize, shape: usize, size: usize) -> Obj {
    Obj {
        y,
        x,
        tile,
        shape,
        size,
        h_flip: false,
        v_flip: false,
        group: None,
        double: false,
    }
}

impl Obj {
    const fn flips(self, h_flip: bool, v_flip: bool) -> Self {
        Self {
            h_flip,
            v_flip,
            ..self
        }
    }
    const fn affine(self, group: usize, double: bool) -> Self {
        Self {
            group: Some(group),
            double,
            ..self
        }
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

fn objects_a() -> Vec<Obj> {
    vec![
        obj(8, 10, 0x01, 0, 2),
        obj(8, 60, 0x22, 0, 1).flips(true, false),
        obj(16, 100, 0x40, 1, 3),
        obj(4, 190, 0x05, 0, 1).affine(2, true),
        obj(70, 10, 0x11, 1, 2),
        obj(70, 60, 0x3F, 0, 0).flips(false, true),
        obj(90, 100, 0x20, 0, 2),
        obj(130, 10, 3, 0, 2),
        obj(130, 60, 40, 2, 2),
        obj(140, 120, 80, 0, 2).affine(3, false),
    ]
}

fn objects_b() -> Vec<Obj> {
    vec![
        obj(10, 10, 4, 0, 1),
        obj(20, 40, 20, 0, 3),
        obj(20, 150, 100, 1, 2).flips(true, true),
        obj(100, 120, 120, 0, 2).affine(1, true),
        obj(150, 200, 7, 0, 0),
        obj(160, -8, 9, 0, 1),
    ]
}

/// Engine A's mapping at screen line `y` (the ROM's DISPCNT bands).
fn mapping_a(y: usize) -> Mapping {
    match y {
        0..=63 => Mapping::TwoD(256),
        64..=127 => Mapping::TwoD(128),
        _ => Mapping::OneD(256),
    }
}

fn mapping_b(_y: usize) -> Mapping {
    Mapping::OneD(128)
}

/// Parameter group `group`: PA, PB, PC, PD (arm9.s, the group tables).
fn parameters(group: usize) -> [i32; 4] {
    match group {
        1 => [0, 256, -256, 0],
        2 => [222, 128, -128, 222],
        3 => [128, 0, 0, 128],
        _ => [0, 0, 0, 0],
    }
}

// ------------------------------------------------------------------ the note's rules

/// The halfword of pixel (`column`, `row`) of the object's picture, `width` pixels across.
fn picture_half(o: &Obj, mapping: Mapping, column: usize, row: usize) -> u16 {
    let (width, _) = o.dimensions();
    let byte = match mapping {
        Mapping::TwoD(source_width) => {
            let mask = source_width / 8 - 1;
            let first = (o.tile & mask) * 0x10 + (o.tile & !mask) * 0x80;
            first + row * source_width * 2 + column * 2
        }
        Mapping::OneD(boundary) => o.tile * boundary + row * width * 2 + column * 2,
    };
    vram_half(byte)
}

/// The 15-bit colour of screen pixel (`x`, `y`) given by one object, or `None`.
fn object_color(o: &Obj, mapping: Mapping, x: usize, y: usize) -> Option<u16> {
    let (width, height) = o.dimensions();
    let (box_width, box_height) = if o.double {
        (2 * width, 2 * height)
    } else {
        (width, height)
    };
    let row = (y + 256 - o.y) % 256;
    let column = x as i32 - o.x;
    if row >= box_height || column < 0 || column >= box_width as i32 {
        return None;
    }
    let (column, row) = (column as usize, row as i32);
    let (u, v) = match o.group {
        Some(group) => {
            let [pa, pb, pc, pd] = parameters(group);
            let dx = column as i32 - (box_width / 2) as i32;
            let dy = row - (box_height / 2) as i32;
            (
                ((pa * dx + pb * dy) >> 8) + (width / 2) as i32,
                ((pc * dx + pd * dy) >> 8) + (height / 2) as i32,
            )
        }
        None => (
            if o.h_flip {
                (width - 1 - column) as i32
            } else {
                column as i32
            },
            if o.v_flip {
                (height as i32) - 1 - row
            } else {
                row
            },
        ),
    };
    if !(0..width as i32).contains(&u) || !(0..height as i32).contains(&v) {
        return None;
    }
    let half = picture_half(o, mapping, u as usize, v as usize);
    (half & 0x8000 != 0).then_some(half & 0x7FFF)
}

fn expected_pixel(
    objects: &[Obj],
    mapping: fn(usize) -> Mapping,
    backdrop: u32,
    x: usize,
    y: usize,
) -> u32 {
    objects
        .iter()
        .find_map(|o| object_color(o, mapping(y), x, y))
        .map_or(backdrop, expand)
}

fn picture_mismatches(
    machine: &Machine,
    screen: Screen,
    objects: &[Obj],
    mapping: fn(usize) -> Mapping,
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
fn given_objbmp_rom_when_5_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(&machine, Screen::Top, &objects_a(), mapping_a, BACKDROP_A);
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_objbmp_rom_when_5_frames_run_then_every_pixel_of_the_lower_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(
        &machine,
        Screen::Bottom,
        &objects_b(),
        mapping_b,
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
fn given_objbmp_rom_when_5_frames_run_then_the_pictures_are_not_blank_and_have_transparent_holes() {
    // The model's own count of non-backdrop pixels must be what the screens show; every object is
    // partly transparent, so the count is below the area of the boxes.
    let machine = boot_frames();
    let count = |objects: &[Obj], mapping: fn(usize) -> Mapping, backdrop: u32| {
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
        count(&objects_a(), mapping_a, BACKDROP_A),
        count(&objects_b(), mapping_b, BACKDROP_B),
    );
    assert_eq!(
        (
            a > 4000 && a < 8000,
            b > 4000 && b < 8000,
            shown(Screen::Top, BACKDROP_A) == a,
            shown(Screen::Bottom, BACKDROP_B) == b
        ),
        (true, true, true, true)
    );
}

#[test]
fn given_objbmp_rom_when_5_frames_run_then_oam_holds_bitmap_objects_with_the_alpha_field_in_attr2()
{
    let machine = boot_frames();
    let view = machine.inspect();
    let (a3, b4) = (view.obj(EngineId::A, 3), view.obj(EngineId::B, 4));
    assert_eq!(
        (
            (a3.mode, a3.affine, a3.double_size, a3.group),
            (b4.mode, b4.palette, b4.tile)
        ),
        ((3, true, true, 2), (3, 0xF, 7))
    );
}

#[test]
fn given_objbmp_rom_when_run_for_5_frames_then_both_screens_match_golden() {
    assert_frame_hash("objbmp", 5);
}

#[test]
fn given_objbmp_rom_when_run_twice_for_5_frames_then_every_frame_hash_matches() {
    assert_deterministic("objbmp", 5);
}

#[test]
fn given_objbmp_rom_when_saved_after_3_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("objbmp", 3, 2);
}
