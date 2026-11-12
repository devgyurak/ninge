// SPDX-License-Identifier: Apache-2.0
//! The OBJ window, observed through our own `objwin` test ROM (`testroms/objwin/`). Four solid text
//! layers (BG0 red in front to BG3 yellow behind) over a white backdrop are shown on both engines;
//! objects of mode 2 choose which layers a pixel shows. Engine A lists a plain, an affine and a
//! half-transparent mode 2 object, two ordinary sprites and window 1 over one of the masks; engine B
//! lists a plain and a mosaic mode 2 object.
//!
//! The expected picture is worked out here, pixel by pixel, from `docs/hw/windows.md` ("The four
//! regions", "The OBJ window") and `docs/hw/obj.md` and the data the ROM's source lists. It is never
//! read back from the emulator: the code below shares nothing with `ninge-core`.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run before the screens are inspected; the ROM finishes its setup inside the first three.
const FRAMES: u64 = 4;

/// A machine that booted the `objwin` ROM and ran `FRAMES` frames.
fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("objwin")).expect("objwin ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("objwin ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame());
    machine
}

// ------------------------------------------------------------------ the data of the ROM

const BACKDROP: u16 = 0x7FFF;
/// BG0-BG3 (arm9.s, "Text layers" of the `win` ROM) and the objects' magenta.
const LAYERS: [u16; 4] = [0x001F, 0x03E0, 0x7C00, 0x03FF];
const MAGENTA: u16 = 0x7C1F;

/// The 15-bit color as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// A rectangle of columns `x.0..x.1` and rows `y.0..y.1`.
#[derive(Clone, Copy)]
struct Rect {
    x: (usize, usize),
    y: (usize, usize),
}

impl Rect {
    fn contains(self, x: usize, y: usize) -> bool {
        (self.x.0..self.x.1).contains(&x) && (self.y.0..self.y.1).contains(&y)
    }
}

/// The columns and rows of a box of `size` pixels at (`x`, `y`).
const fn square(x: usize, y: usize, size: usize) -> Rect {
    Rect {
        x: (x, x + size),
        y: (y, y + size),
    }
}

/// What a scene shows at a pixel.
struct Scene {
    /// Window 1, when on: its rectangle and the layers inside.
    window1: Option<(Rect, u8)>,
    /// The layers inside the OBJ window and outside every window; bit n is BGn, bit 4 the OBJ layer.
    obj_window: u8,
    outside: u8,
    /// Whether the pixel is an OBJ window pixel.
    mask: fn(usize, usize) -> bool,
    /// Whether an ordinary sprite has an opaque pixel there.
    sprite: fn(usize, usize) -> bool,
}

impl Scene {
    /// The layers the pixel's region shows: window 1, else the OBJ window, else the outside.
    fn group(&self, x: usize, y: usize) -> u8 {
        match self.window1 {
            Some((rect, group)) if rect.contains(x, y) => group,
            _ if (self.mask)(x, y) => self.obj_window,
            _ => self.outside,
        }
    }

    fn color(&self, x: usize, y: usize) -> u32 {
        let group = self.group(x, y);
        let layer = (0..4)
            .find(|n| group >> n & 1 != 0)
            .map_or(BACKDROP, |n| LAYERS[n]);
        // An ordinary sprite is in front of equal-priority layers, where the region shows objects.
        expand(match group & 0x10 != 0 && (self.sprite)(x, y) {
            true => MAGENTA,
            false => layer,
        })
    }
}

/// Engine A's mask (arm9.s, entries 4, 5, 7 and 8).
fn mask_a(x: usize, y: usize) -> bool {
    // Entry 5: the box at (80, 40) with PA = PD = 2.0 shows its middle 16 by 16.
    let affine = square(88, 48, 16);
    // Entry 8: the left four columns of an 8x8 tile are opaque.
    let half = Rect {
        x: (200, 204),
        y: (100, 108),
    };
    [square(20, 20, 32), affine, square(150, 50, 32), half]
        .iter()
        .any(|rect| rect.contains(x, y))
}

/// Engine A's ordinary sprites (entries 6 and 9).
fn sprite_a(x: usize, y: usize) -> bool {
    [square(40, 30, 16), square(202, 102, 8)]
        .iter()
        .any(|rect| rect.contains(x, y))
}

fn scene_a() -> Scene {
    Scene {
        window1: Some((
            Rect {
                x: (120, 180),
                y: (20, 80),
            },
            0b01000,
        )),
        obj_window: 0b00010,
        outside: 0b10001,
        mask: mask_a,
        sprite: sprite_a,
    }
}

/// Engine B's mask (arm9.s, entries 0 and 1). Entry 1 is a 16x16 mosaic object at X 30 with blocks 4
/// wide and 1 high: a block that starts left of the object is transparent, and the object's columns
/// run to the end of the last block that starts inside its box (`obj.md`, "Mosaic").
fn mask_b(x: usize, y: usize) -> bool {
    let block = x - x % 4;
    let mosaic = (30..46).contains(&block) && (150..166).contains(&y);
    square(100, 100, 32).contains(x, y) || mosaic
}

fn scene_b() -> Scene {
    Scene {
        window1: None,
        obj_window: 0b00100,
        outside: 0b00001,
        mask: mask_b,
        sprite: |_, _| false,
    }
}

/// The (x, y, got, want) of the first pixels of `screen` that differ from the scene.
fn misses(machine: &Machine, screen: Screen, scene: &Scene) -> Vec<(usize, usize, u32, u32)> {
    let frame = machine.frame(screen);
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .filter_map(|(x, y)| {
            let (got, want) = (frame[y * SCREEN_WIDTH + x], scene.color(x, y));
            (got != want).then_some((x, y, got, want))
        })
        .take(8)
        .collect()
}

// ------------------------------------------------------------------------------ the tests

#[test]
fn given_objwin_rom_when_4_frames_run_then_the_upper_screen_shows_the_obj_window_and_window_1() {
    assert_eq!(misses(&boot_frames(), Screen::Top, &scene_a()), vec![]);
}

#[test]
fn given_objwin_rom_when_4_frames_run_then_the_lower_screen_shows_a_plain_and_a_mosaic_mask() {
    assert_eq!(misses(&boot_frames(), Screen::Bottom, &scene_b()), vec![]);
}

#[test]
fn given_objwin_rom_when_4_frames_run_then_each_region_shows_its_own_layer() {
    // A guard against a scene that went blank: inside entry 4's mask BG1 (green), inside window 1
    // BG3 (yellow), an ordinary sprite outside both is magenta and the rest is BG0 (red); on the lower
    // screen the mask shows BG2 (blue) and the rest BG0.
    let machine = boot_frames();
    let (top, bottom) = (machine.frame(Screen::Top), machine.frame(Screen::Bottom));
    let at = |frame: &[u32], x: usize, y: usize| frame[y * SCREEN_WIDTH + x];
    assert_eq!(
        (
            at(top, 25, 25),
            at(top, 130, 30),
            at(top, 54, 40),
            at(top, 5, 5),
            at(bottom, 110, 110),
            at(bottom, 5, 5),
        ),
        (
            expand(LAYERS[1]),
            expand(LAYERS[3]),
            expand(MAGENTA),
            expand(LAYERS[0]),
            expand(LAYERS[2]),
            expand(LAYERS[0]),
        )
    );
}

#[test]
fn given_objwin_rom_when_run_for_4_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that this frame is the right one: the hash pins it.
    assert_frame_hash("objwin", 4);
}

#[test]
fn given_objwin_rom_when_run_twice_for_4_frames_then_every_frame_hash_matches() {
    assert_deterministic("objwin", 4);
}

#[test]
fn given_objwin_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("objwin", 2, 2);
}
