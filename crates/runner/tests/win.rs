// SPDX-License-Identifier: Apache-2.0
//! Windows, observed through our own `win` test ROM (`testroms/win/`). Four solid text layers (BG0 red
//! in front to BG3 yellow behind) over a white backdrop are shown on both engines; engine A has windows 0
//! and 1 overlapping, each with its own layer, and BG3 outside, engine B has window 1 alone with its X1
//! and Y1 past X2 and Y2 and BG1 and BG2 outside.
//!
//! The expected picture is worked out here, pixel by pixel, from `docs/hw/windows.md` and the data the
//! ROM's source lists. It is never read back from the emulator: the code below shares nothing with
//! `ninge-core`.

use ninge_core::debug::EngineId;
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run before the screens are inspected; the ROM finishes its setup inside the first three.
const FRAMES: u64 = 4;

/// A machine that booted the `win` ROM and ran `FRAMES` frames.
fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("win")).expect("win ROM is built");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("win ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame());
    machine
}

// ------------------------------------------------------------------ the data of the ROM

/// The palette of both engines (arm9.s, "Palette"): the backdrop, then BG0-BG3.
const BACKDROP: u16 = 0x7FFF;
const LAYERS: [u16; 4] = [0x001F, 0x03E0, 0x7C00, 0x03FF];

/// The 15-bit color as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// A rectangle as the ROM's registers hold it: X1 and X2, Y1 and Y2 (`windows.md`, "The
/// rectangles"). The end is clamped to the screen when it is past it or before the start.
#[derive(Clone, Copy)]
struct Rect {
    x: (usize, usize),
    y: (usize, usize),
}

impl Rect {
    fn contains(self, x: usize, y: usize) -> bool {
        let end = |(start, end): (usize, usize), limit: usize| match end > limit || start > end {
            true => limit,
            false => end,
        };
        (self.x.0..end(self.x, SCREEN_WIDTH)).contains(&x)
            && (self.y.0..end(self.y, SCREEN_HEIGHT)).contains(&y)
    }
}

/// The layers (bit n = BGn) a pixel shows in a scene: windows 0 and 1 (rectangle, whether on, group)
/// and the outside group.
struct Scene {
    windows: [(Rect, bool, u8); 2],
    outside: u8,
}

impl Scene {
    fn group(&self, x: usize, y: usize) -> u8 {
        self.windows
            .iter()
            .find(|(rect, on, _)| *on && rect.contains(x, y))
            .map_or(self.outside, |&(_, _, group)| group)
    }

    /// The colour at a pixel: the front-most layer of its group, else the backdrop.
    fn color(&self, x: usize, y: usize) -> u32 {
        let group = self.group(x, y);
        expand(
            (0..4)
                .find(|n| group >> n & 1 != 0)
                .map_or(BACKDROP, |n| LAYERS[n]),
        )
    }
}

/// Engine A (arm9.s): WIN0H 0x28A0, WIN0V 0x1E78, WIN1H 0x64DC, WIN1V 0x50AA, WININ 0x0402, WINOUT
/// 0x0008, both windows on.
fn scene_a() -> Scene {
    Scene {
        windows: [
            (
                Rect {
                    x: (40, 160),
                    y: (30, 120),
                },
                true,
                0b0010,
            ),
            (
                Rect {
                    x: (100, 220),
                    y: (80, 170),
                },
                true,
                0b0100,
            ),
        ],
        outside: 0b1000,
    }
}

/// Engine B (arm9.s): WIN1H 0xB43C (X1 180, X2 60), WIN1V 0x6428 (Y1 100, Y2 40), WININ 0x0100,
/// WINOUT 0x0006, window 1 on and window 0 off.
fn scene_b() -> Scene {
    Scene {
        windows: [
            (
                Rect {
                    x: (0, 0),
                    y: (0, 0),
                },
                false,
                0,
            ),
            (
                Rect {
                    x: (180, 60),
                    y: (100, 40),
                },
                true,
                0b0001,
            ),
        ],
        outside: 0b0110,
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
fn given_win_rom_when_4_frames_run_then_the_upper_screen_shows_the_regions_of_both_windows() {
    assert_eq!(misses(&boot_frames(), Screen::Top, &scene_a()), vec![]);
}

#[test]
fn given_win_rom_when_4_frames_run_then_the_lower_screen_shows_window_1_clamped_to_the_edges() {
    assert_eq!(misses(&boot_frames(), Screen::Bottom, &scene_b()), vec![]);
}

#[test]
fn given_win_rom_when_4_frames_run_then_the_overlap_of_the_windows_shows_window_0s_layer() {
    // A guard against a scene that went blank: the overlap (columns 100-159, rows 80-119) is
    // window 0's green, a pixel in window 1 alone is blue, and one in neither is yellow.
    let machine = boot_frames();
    let top = machine.frame(Screen::Top);
    let at = |x: usize, y: usize| top[y * SCREEN_WIDTH + x];
    assert_eq!(
        (at(120, 100), at(200, 150), at(5, 5)),
        (expand(LAYERS[1]), expand(LAYERS[2]), expand(LAYERS[3]))
    );
}

#[test]
fn given_win_rom_when_4_frames_run_then_the_rom_wrote_the_registers_it_set() {
    let machine = boot_frames();
    let gpu = machine.inspect().gpu2d();
    let (a, b) = (gpu.windows(EngineId::A), gpu.windows(EngineId::B));
    let rects =
        |w: &ninge_core::debug::Windows| w.rects.map(|r| (r.left, r.right, r.top, r.bottom));
    assert_eq!(
        (
            gpu.dispcnt_a(),
            gpu.dispcnt_b(),
            rects(&a),
            a.enabled,
            rects(&b)[1],
            b.enabled,
        ),
        (
            0x0001_6F00,
            0x0001_4F00,
            [(40, 160, 30, 120), (100, 220, 80, 170)],
            [true, true, false],
            (180, 256, 100, 192),
            [false, true, false],
        )
    );
}

#[test]
fn given_win_rom_when_run_for_4_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that this frame is the right one: the hash pins it.
    assert_frame_hash("win", 4);
}

#[test]
fn given_win_rom_when_run_twice_for_4_frames_then_every_frame_hash_matches() {
    assert_deterministic("win", 4);
}

#[test]
fn given_win_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("win", 2, 2);
}
