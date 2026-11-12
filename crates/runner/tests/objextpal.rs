// SPDX-License-Identifier: Apache-2.0
//! OBJ extended palettes on both engines, observed through our own `objextpal` test ROM
//! (`testroms/objextpal/`). The ROM fills banks F, G and I with computed extended palettes while
//! they are in LCDC mode, maps F (engine A) and I (engine B), and lists the same eight 32x32 objects
//! for each engine in two bands (lines 10-41 and 110-141): 256-colour objects with different
//! palette numbers, a 16-colour object (standard palette) and an affine 256-colour object (a quarter
//! turn). By polling VCOUNT the ROM moves engine A from bank F to bank G at line 96 and clears
//! engine B's DISPCNT bit 31 at line 96, so one frame shows both banks and the standard palette.
//!
//! The expected picture is worked out here, pixel by pixel, from `docs/hw/extended-palettes.md`
//! ("Objects"), `docs/hw/obj.md` and the formulas at the top of the ROM's `arm9.s`. It is never read
//! back from the emulator: the code below shares nothing with `ninge-core`.

use ninge_core::debug::EngineId;
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const FRAMES: u64 = 6;

fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("objextpal")).expect("objextpal ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("objextpal ROM has a valid header");
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

const BACKDROP_A: u32 = 0x0000_FF00;

/// The constants K of the extended palette banks F, G and I (arm9.s, "OBJ extended palette colours").
const K_F: usize = 0x1A3;
const K_G: usize = 0x3C5;
const K_I: usize = 0x2E1;

/// The extended colour of palette `palette`, entry `index` in a bank of constant `k`.
fn extended(k: usize, palette: usize, index: usize) -> u16 {
    ((palette * 0xB3 + index * 0x47 + k) & 0x7FFF) as u16
}
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
    const fn plain(self) -> Self {
        Self {
            group: None,
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

/// The eight objects of both engines (arm9.s): 32x32, 2D mapping, tile 0x40.
fn objects() -> Vec<Obj> {
    let band = |y: usize, palettes: [usize; 4]| {
        [
            affine(y, 10, 0x40, 0, 2, 0)
                .plain()
                .colors256()
                .palette(palettes[0]),
            affine(y, 60, 0x40, 0, 2, 0)
                .plain()
                .colors256()
                .palette(palettes[1]),
            affine(y, 110, 0x40, 0, 2, 0).plain().palette(palettes[2]),
            affine(y, 160, 0x40, 0, 2, 1)
                .colors256()
                .palette(palettes[3]),
        ]
    };
    let mut all = band(10, [2, 9, 3, 5]).to_vec();
    all.extend(band(110, [11, 14, 6, 12]));
    all
}

/// Parameter group `group`: PA, PB, PC, PD (arm9.s, "Parameter groups").
fn parameters(group: usize) -> [i32; 4] {
    match group {
        1 => [0, 256, -256, 0],
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

/// Which palette the object's pixel (`value`) reads, as a 15-bit colour: `ext` is the extended bank
/// constant that is on for this pixel's line, or `None` when bit 31 / the bank is off.
fn pixel_color(o: &Obj, value: usize, ext: Option<usize>) -> Option<u16> {
    match (value, o.eight_bpp, ext) {
        (0, _, _) => None,
        (value, true, Some(k)) => Some(extended(k, o.palette, value)),
        (value, true, None) => Some(standard(value)),
        (value, false, _) => Some(standard(o.palette * 16 + value)),
    }
}

/// The standard OBJ palette entry `index` (arm9.s, "Standard OBJ palettes").
fn standard(index: usize) -> u16 {
    let red = index & 31;
    let green = (3 * index) & 31;
    let blue = (5 * index) & 31;
    (red | green << 5 | blue << 10) as u16
}

/// The 15-bit colour of screen pixel (`x`, `y`) given by one object, or `None`. A plain object shows
/// its picture in its box; an affine one (group 1) is sampled through the quarter turn.
fn object_color(o: &Obj, ext: Option<usize>, x: usize, y: usize) -> Option<u16> {
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
        None => (column as i32, row as i32),
    };
    if !(0..width as i32).contains(&u) || !(0..height as i32).contains(&v) {
        return None;
    }
    pixel_color(o, tile_value(o, None, u as usize, v as usize), ext)
}

/// The expected pixel of engine A: bank F's palettes for lines 0-95, bank G's after.
fn expected_a(x: usize, y: usize) -> u32 {
    let k = if y < 96 { K_F } else { K_G };
    objects()
        .iter()
        .find_map(|o| object_color(o, Some(k), x, y))
        .map_or(BACKDROP_A, expand)
}

/// The expected pixel of engine B: bank I's palettes for lines 0-95, the standard palette after.
fn expected_b(x: usize, y: usize) -> u32 {
    let ext = if y < 96 { Some(K_I) } else { None };
    objects()
        .iter()
        .find_map(|o| object_color(o, ext, x, y))
        .map_or(BACKDROP_B, expand)
}

fn picture_mismatches(
    machine: &Machine,
    screen: Screen,
    expected: fn(usize, usize) -> u32,
) -> Vec<(usize, usize, u32, u32)> {
    let frame = machine.frame(screen);
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| (x, y, expected(x, y), frame[y * SCREEN_WIDTH + x]))
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

// ------------------------------------------------------------------ the tests

#[test]
fn given_objextpal_rom_when_6_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(&machine, Screen::Top, expected_a);
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_objextpal_rom_when_6_frames_run_then_every_pixel_of_the_lower_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(&machine, Screen::Bottom, expected_b);
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_objextpal_rom_when_6_frames_run_then_the_extended_palettes_make_a_difference() {
    // Guards the two tests above against a ROM whose extended palettes did nothing. Object 0 (band
    // 1, palette 2) and object 4 (band 2, palette 11) are 256-colour objects whose pixel (3, 3) has
    // the same value.
    let machine = boot_frames();
    let (top, bottom) = (machine.frame(Screen::Top), machine.frame(Screen::Bottom));
    let at = |frame: &[u32], x: usize, y: usize| frame[y * SCREEN_WIDTH + x];
    let value = tile_value(&objects()[0], None, 3, 3);
    assert_eq!(
        (
            // Engine A, bank F in the first band, not the standard palette.
            at(top, 13, 13) == expand(extended(K_F, 2, value)),
            at(top, 13, 13) != expand(standard(value)),
            // Its second band reads bank G, not F.
            at(top, 13, 113) == expand(extended(K_G, 11, value)),
            at(top, 13, 113) != expand(extended(K_F, 11, value)),
            // Engine B: bank I in the first band, the standard palette after bit 31 is cleared.
            at(bottom, 13, 13) == expand(extended(K_I, 2, value)),
            at(bottom, 13, 113) == expand(standard(value)),
        ),
        (true, true, true, true, true, true)
    );
}

#[test]
fn given_objextpal_rom_when_6_frames_run_then_the_slots_report_the_banks_of_the_frame_start() {
    let machine = boot_frames();
    let view = machine.inspect();
    let (a, b) = (
        view.ext_palette_banks(EngineId::A),
        view.ext_palette_banks(EngineId::B),
    );
    // Bank F (index 5) backs engine A's OBJ slot and bank I (index 8) engine B's.
    assert_eq!(
        (a.obj, b.obj, a.bg, b.bg),
        (Some(5), Some(8), [None; 4], [None; 4])
    );
}

#[test]
fn given_objextpal_rom_when_run_for_6_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that these frames are the right ones: the hash pins
    // them.
    assert_frame_hash("objextpal", 6);
}

#[test]
fn given_objextpal_rom_when_run_twice_for_6_frames_then_every_frame_hash_matches() {
    assert_deterministic("objextpal", 6);
}

#[test]
fn given_objextpal_rom_when_saved_after_4_frames_and_resumed_then_it_matches_an_uninterrupted_run()
{
    assert_savestate_round_trip("objextpal", 4, 2);
}
