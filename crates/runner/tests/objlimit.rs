// SPDX-License-Identifier: Apache-2.0
//! The per-line object budget on both engines, observed through our own `objlimit` test ROM
//! (`testroms/objlimit/`). The ROM crowds lines 60-71 with objects that are entirely off the
//! screen (they cost their rendering cycles all the same) in front of visible 8x32 objects in OAM
//! order, so that the visible objects behind them are dropped: engine A has the default budget of
//! 1210 cycles a line and two visible objects fit, engine B has 954 (DISPCNT bit 23) and seven
//! do. On lines 40-59 the off-screen objects are not on the line, so every visible object shows.
//!
//! The expected pictures are worked out here, pixel by pixel, from `docs/hw/obj.md` ("The line
//! limit"): a plain object costs its box width, an affine one 10 plus twice it; objects are looked at
//! in entry order, and the first one that does not fit and every later one are dropped. They are
//! never read back from the emulator: the code below shares nothing with `ninge-core`.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const FRAMES: u64 = 3;

fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("objlimit")).expect("objlimit ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("objlimit ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame());
    machine
}

// ------------------------------------------------------------------ the data of the ROM

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
    shape: usize,
    size: usize,
    palette: usize,
    /// An affine object with group 0 (the identity) and its double-size bit.
    affine: Option<bool>,
}

const fn obj(y: usize, x: i32, tile: usize, shape: usize, size: usize, palette: usize) -> Obj {
    Obj {
        y,
        x,
        tile,
        shape,
        size,
        palette,
        affine: None,
    }
}

impl Obj {
    fn dimensions(&self) -> (usize, usize) {
        const TABLE: [[(usize, usize); 4]; 3] = [
            [(8, 8), (16, 16), (32, 32), (64, 64)],
            [(16, 8), (32, 8), (32, 16), (64, 32)],
            [(8, 16), (8, 32), (16, 32), (32, 64)],
        ];
        TABLE[self.shape][self.size]
    }

    /// The box: twice the size for a double-size affine object.
    fn box_size(&self) -> (usize, usize) {
        let (width, height) = self.dimensions();
        match self.affine {
            Some(true) => (2 * width, 2 * height),
            _ => (width, height),
        }
    }

    /// The cost in cycles (`obj.md`, "The line limit").
    fn cost(&self) -> usize {
        match self.affine {
            Some(_) => 10 + 2 * self.box_size().0,
            None => self.box_size().0,
        }
    }
}

/// The off-screen 64x32 plain object of the ROM at line 60.
const fn big() -> Obj {
    obj(60, -256, 0, 1, 3, 0)
}

/// The off-screen 32x32 double-size affine object of the ROM at line 60.
const fn affine_big() -> Obj {
    Obj {
        affine: Some(true),
        ..obj(60, -256, 0, 0, 2, 0)
    }
}

/// A visible 8x32 object at line 40.
const fn tall(x: i32, tile: usize, palette: usize) -> Obj {
    obj(40, x, tile, 2, 1, palette)
}

fn objects_a() -> Vec<Obj> {
    let mut objects = vec![big(); 12];
    objects.extend([affine_big(); 3]);
    objects.extend([
        tall(10, 1, 1),
        tall(24, 5, 2),
        obj(40, 40, 20, 1, 3, 3),
        tall(120, 30, 4),
        tall(134, 34, 5),
        tall(148, 38, 6),
    ]);
    objects
}

fn objects_b() -> Vec<Obj> {
    let mut objects = vec![big(); 14];
    objects.extend((0..9).map(|k| tall(10 + 14 * k as i32, k + 1, k + 1)));
    objects
}

// ------------------------------------------------------------------ the note's rules

/// Which mapping a scene uses: `Some(boundary)` for 1D with that many bytes, `None` for 2D.
type Mapping = Option<usize>;

fn tile_value(o: &Obj, mapping: Mapping, column: usize, row: usize) -> usize {
    let (width, _) = o.dimensions();
    let (tile_column, tile_row, x, y) = (column / 8, row / 8, column % 8, row % 8);
    let tile_at = match mapping {
        Some(boundary) => o.tile * boundary + (tile_column + tile_row * (width / 8)) * 32,
        None => (o.tile + tile_column + tile_row * 32) * 32,
    };
    let byte = vram_byte(tile_at + y * 4 + x / 2);
    usize::from(if x % 2 == 0 { byte & 0xF } else { byte >> 4 })
}

/// The palette index of screen pixel (`x`, `y`) given by one object, or `None`. An affine object
/// here has the identity, so its picture is the plain one in the middle of the box.
fn object_index(o: &Obj, mapping: Mapping, x: usize, y: usize) -> Option<usize> {
    let (width, height) = o.dimensions();
    let (box_width, box_height) = o.box_size();
    let row = (y + 256 - o.y) % 256;
    let column = usize::try_from(x as i32 - o.x).ok()?;
    if row >= box_height || column >= box_width {
        return None;
    }
    let (u, v) = (
        column as i32 - (box_width / 2) as i32 + (width / 2) as i32,
        row as i32 - (box_height / 2) as i32 + (height / 2) as i32,
    );
    if !(0..width as i32).contains(&u) || !(0..height as i32).contains(&v) {
        return None;
    }
    match tile_value(o, mapping, u as usize, v as usize) {
        0 => None,
        value => Some(o.palette * 16 + value),
    }
}

/// The objects line `y` draws under a budget of `budget` cycles: those on the line, in entry order,
/// until one does not fit.
fn drawn_on_line(objects: &[Obj], budget: usize, y: usize) -> Vec<Obj> {
    let mut left = budget;
    let mut drawn = Vec::new();
    for o in objects {
        let on_line = (y + 256 - o.y) % 256 < o.box_size().1;
        if !on_line {
            continue;
        }
        match left.checked_sub(o.cost()) {
            Some(rest) => {
                left = rest;
                drawn.push(*o);
            }
            None => break,
        }
    }
    drawn
}

fn expected_pixel(
    objects: &[Obj],
    budget: usize,
    mapping: Mapping,
    backdrop: u32,
    x: usize,
    y: usize,
) -> u32 {
    drawn_on_line(objects, budget, y)
        .iter()
        .find_map(|o| object_index(o, mapping, x, y))
        .map_or(backdrop, |index| expand(palette(index)))
}

fn picture_mismatches(
    machine: &Machine,
    screen: Screen,
    objects: &[Obj],
    budget: usize,
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
                expected_pixel(objects, budget, mapping, backdrop, x, y),
                frame[y * SCREEN_WIDTH + x],
            )
        })
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

// ------------------------------------------------------------------ the tests

#[test]
fn given_objlimit_rom_when_3_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(
        &machine,
        Screen::Top,
        &objects_a(),
        1210,
        Some(32),
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
fn given_objlimit_rom_when_3_frames_run_then_every_pixel_of_the_lower_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(
        &machine,
        Screen::Bottom,
        &objects_b(),
        954,
        None,
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
fn given_objlimit_rom_when_3_frames_run_then_the_budget_drops_visible_objects_on_the_crowded_lines_only()
 {
    // Spot checks straight from the note, not through the model: how many of the visible 8x32
    // objects show anything in their 8 columns at line 50 (the costly objects do not reach it)
    // and at line 65 (they do).
    let machine = boot_frames();
    let shown = |screen: Screen, backdrop: u32, xs: &[usize], line: usize| {
        let frame = machine.frame(screen);
        xs.iter()
            .filter(|&&x| (x..x + 8).any(|c| frame[line * SCREEN_WIDTH + c] != backdrop))
            .count()
    };
    let xs_a: Vec<usize> = vec![10, 24, 120, 134, 148];
    let xs_b: Vec<usize> = (0..9).map(|k| 10 + 14 * k).collect();
    assert_eq!(
        [
            // Engine A: entries 15, 16, 18, 19, 20 on a line the costly objects do not reach, then
            // on one they do (1210 - 1182 = 28 left: entries 15 and 16 fit, entry 17 does not).
            shown(Screen::Top, BACKDROP_A, &xs_a, 50),
            shown(Screen::Top, BACKDROP_A, &xs_a, 65),
            // Engine B: nine of nine, then seven of nine (954 - 896 = 58 left: 7 * 8 = 56).
            shown(Screen::Bottom, BACKDROP_B, &xs_b, 50),
            shown(Screen::Bottom, BACKDROP_B, &xs_b, 65),
        ],
        [5, 2, 9, 7]
    );
}

#[test]
fn given_objlimit_rom_when_run_for_3_frames_then_both_screens_match_golden() {
    assert_frame_hash("objlimit", 3);
}

#[test]
fn given_objlimit_rom_when_run_twice_for_3_frames_then_every_frame_hash_matches() {
    assert_deterministic("objlimit", 3);
}

#[test]
fn given_objlimit_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("objlimit", 2, 2);
}
