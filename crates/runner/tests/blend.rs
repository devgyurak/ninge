// SPDX-License-Identifier: Apache-2.0
//! Colour special effects, observed through our own `blend` test ROM (`testroms/blend/`). The four
//! solid text layers of the `win` ROM (BG0 red in front to BG3 yellow behind, over a white backdrop)
//! are cut into three vertical bands by windows, so a band shows other layers and has its own effect
//! bit; engine A has a normal object and a semi-transparent one. The ROM steps through nine
//! BLDCNT/BLDALPHA/BLDY settings for each engine, one for each frame.
//!
//! The expected pictures are worked out here, pixel by pixel, from `docs/hw/blending.md` and the
//! data the ROM's source lists. They are never read back from the emulator: the code below shares
//! nothing with `ninge-core`. Which frame shows which setting is not modelled: the test keeps the
//! pictures in the order they appear, drops the white screens before the ROM sets DISPCNT and any
//! frame equal to the one before, and compares that list with the nine settings in order.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run: the ROM writes one setting a frame, nine in all, after a short setup.
const FRAMES: u64 = 14;

/// Both screens after each frame, in the order the frames were made.
fn pictures() -> Vec<Picture> {
    let rom = load_rom_file(&testrom_path("blend")).expect("blend ROM is built");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("blend ROM has a valid header");
    (0..FRAMES)
        .map(|_| {
            machine.run_frame(ninge_core::Input::new());
            Picture {
                top: machine.frame(Screen::Top).to_vec(),
                bottom: machine.frame(Screen::Bottom).to_vec(),
            }
        })
        .collect()
}

#[derive(Clone, PartialEq, Debug)]
struct Picture {
    top: Vec<u32>,
    bottom: Vec<u32>,
}

/// `list` without a picture equal to the one before it.
fn without_repeats(list: Vec<Picture>) -> Vec<Picture> {
    let mut kept: Vec<Picture> = Vec::new();
    list.into_iter().for_each(|picture| {
        if kept.last() != Some(&picture) {
            kept.push(picture);
        }
    });
    kept
}

// ------------------------------------------------------------------ the data of the ROM

const WHITE: u16 = 0x7FFF;
const MAGENTA: u16 = 0x7C1F;
/// BG0-BG3 (arm9.s, "Palette" of the `win` ROM).
const LAYERS: [u16; 4] = [0x001F, 0x03E0, 0x7C00, 0x03FF];

/// The layer bits of BLDCNT: BG0-BG3 are bits 0-3, the objects bit 4, the backdrop bit 5.
const OBJ_BIT: u8 = 4;
const BACKDROP_BIT: u8 = 5;

/// The 15-bit colour as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// A pixel a layer makes: its colour, the layer bit and whether it is a semi-transparent object.
#[derive(Clone, Copy)]
struct Px {
    color: u16,
    bit: u8,
    semi: bool,
}

/// BLDCNT, BLDALPHA and BLDY (arm9.s, "Steps").
type Regs = [u16; 3];

/// One step: engine A's registers, then engine B's (the rows of `steps`).
const STEPS: [[Regs; 2]; 9] = [
    [[0x0241, 0x0808, 0], [0x0844, 0x0808, 0]],
    [[0x0441, 0x0C04, 0], [0x2068, 0x0C04, 0]],
    [[0x0241, 0x141F, 0], [0x00E8, 0, 8]],
    [[0x0091, 0, 8], [0x0084, 0, 16]],
    [[0x00D1, 0, 8], [0x204C, 0x1010, 0]],
    [[0x0181, 0x0808, 16], [0, 0, 0]],
    [[0x0041, 0x0808, 0], [0x0844, 0x0808, 0]],
    [[0x0150, 0x0808, 0], [0x2068, 0x0C04, 0]],
    [[0x0100, 0, 0], [0x00E8, 0, 8]],
];

/// The layers a band shows (bit n = BGn, bit 4 = objects) and whether it allows the effect.
struct Band {
    layers: u8,
    effect: bool,
}

/// Engine A's band at column `x`: window 0 (columns 0-79), window 1 (80-159) or outside.
fn band_a(x: usize) -> Band {
    match x {
        0..80 => Band {
            layers: 0b1_1111,
            effect: true,
        },
        80..160 => Band {
            layers: 0b1_1101,
            effect: true,
        },
        _ => Band {
            layers: 0b1_0011,
            effect: false,
        },
    }
}

/// Engine B's band at column `x`: window 0 (columns 0-84), window 1 (85-169) or outside.
fn band_b(x: usize) -> Band {
    match x {
        0..85 => Band {
            layers: 0,
            effect: true,
        },
        85..170 => Band {
            layers: 0b1000,
            effect: true,
        },
        _ => Band {
            layers: 0b1100,
            effect: true,
        },
    }
}

/// The object pixel of engine A at (x, y): entry 0 is a normal 8x8 object at (20, 20), entry 1 a
/// semi-transparent one at (100, 20). Objects have priority 0, like the layers, and so are in front.
fn object_a(x: usize, y: usize) -> Option<Px> {
    let rows = (20..28).contains(&y);
    let at = |left: usize| (left..left + 8).contains(&x) && rows;
    [(20, false), (100, true)]
        .into_iter()
        .find(|&(left, _)| at(left))
        .map(|(_, semi)| Px {
            color: MAGENTA,
            bit: OBJ_BIT,
            semi,
        })
}

/// The pixels at (x, y) from the front: the object, the layers the band shows, the backdrop.
fn stack(band: &Band, object: Option<Px>) -> Vec<Px> {
    let objects = object.filter(|_| band.layers >> OBJ_BIT & 1 != 0);
    let layers = (0..4u8).filter(|n| band.layers >> n & 1 != 0).map(|n| Px {
        color: LAYERS[usize::from(n)],
        bit: n,
        semi: false,
    });
    let backdrop = Px {
        color: WHITE,
        bit: BACKDROP_BIT,
        semi: false,
    };
    objects
        .into_iter()
        .chain(layers)
        .chain(std::iter::once(backdrop))
        .collect()
}

/// A colour's channels, red first.
fn channels(color: u16) -> [u32; 3] {
    [color & 31, color >> 5 & 31, color >> 10 & 31].map(u32::from)
}

fn join([r, g, b]: [u32; 3]) -> u16 {
    (r | g << 5 | b << 10) as u16
}

/// The colour shown where `stack` is the pixels from the front, under the registers `regs`
/// (`blending.md`, "Behavior"): sixteenths, truncated, five-bit channels.
fn shown(regs: Regs, stack: &[Px], allowed: bool) -> u16 {
    let [bldcnt, bldalpha, bldy] = regs;
    let (top, below) = (stack[0], stack.get(1));
    let effect = bldcnt >> 6 & 3;
    let (first, second) = (bldcnt & 0x3F, bldcnt >> 8 & 0x3F);
    let coefficient = |value: u16| u32::from((value & 31).min(16));
    let (eva, evb, evy) = (
        coefficient(bldalpha),
        coefficient(bldalpha >> 8),
        coefficient(bldy),
    );
    // A semi-transparent object is a first target whatever BLDCNT says.
    let is_first = top.semi || first >> top.bit & 1 != 0;
    if !allowed || !is_first {
        return top.color;
    }
    let second_below = below.is_some_and(|px| second >> px.bit & 1 != 0);
    match (effect, second_below) {
        (1, true) | (_, true) if top.semi || effect == 1 => {
            let (a, b) = (
                channels(top.color),
                channels(below.expect("a second").color),
            );
            join([0, 1, 2].map(|i| ((a[i] * eva + b[i] * evb) / 16).min(31)))
        }
        (2, _) => join(channels(top.color).map(|c| c + (31 - c) * evy / 16)),
        (3, _) => join(channels(top.color).map(|c| c - c * evy / 16)),
        _ => top.color,
    }
}

/// Engine A's picture under `regs`, as xRGB8888.
fn screen_a(regs: Regs) -> Vec<u32> {
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| {
            let band = band_a(x);
            expand(shown(regs, &stack(&band, object_a(x, y)), band.effect))
        })
        .collect()
}

/// Engine B's picture under `regs`: no objects.
fn screen_b(regs: Regs) -> Vec<u32> {
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |_| y))
        .enumerate()
        .map(|(i, _)| {
            let band = band_b(i % SCREEN_WIDTH);
            expand(shown(regs, &stack(&band, None), band.effect))
        })
        .collect()
}

/// The pictures the ROM goes through: no effect, then each step. Engine A is on the upper screen.
fn expected() -> Vec<Picture> {
    let picture = |a: Regs, b: Regs| Picture {
        top: screen_a(a),
        bottom: screen_b(b),
    };
    std::iter::once(picture([0; 3], [0; 3]))
        .chain(STEPS.iter().map(|&[a, b]| picture(a, b)))
        .collect()
}

/// The (picture index, screen, x, y, got, want) of the first pixels that differ.
fn misses(got: &[Picture], want: &[Picture]) -> Vec<(usize, &'static str, usize, usize, u32, u32)> {
    got.iter()
        .zip(want)
        .enumerate()
        .flat_map(|(i, (got, want))| {
            [
                ("top", &got.top, &want.top),
                ("bottom", &got.bottom, &want.bottom),
            ]
            .into_iter()
            .flat_map(move |(name, got, want)| {
                got.iter()
                    .zip(want)
                    .enumerate()
                    .filter_map(move |(p, (&g, &w))| {
                        (g != w).then_some((i, name, p % SCREEN_WIDTH, p / SCREEN_WIDTH, g, w))
                    })
            })
        })
        .take(8)
        .collect()
}

/// The pictures the ROM shows, without the white ones before DISPCNT is set and without repeats.
fn shown_pictures() -> Vec<Picture> {
    let white = expand(WHITE);
    without_repeats(
        pictures()
            .into_iter()
            .filter(|p| !p.top.iter().all(|&px| px == white))
            .collect(),
    )
}

// ------------------------------------------------------------------------------ the tests

#[test]
fn given_blend_rom_when_run_then_it_goes_through_the_plain_picture_and_nine_settings() {
    assert_eq!(shown_pictures().len(), expected().len());
}

#[test]
fn given_blend_rom_when_run_then_each_picture_of_both_engines_matches_the_note() {
    assert_eq!(misses(&shown_pictures(), &expected()), vec![]);
}

#[test]
fn given_blend_rom_when_run_then_the_pictures_differ_from_each_other() {
    // A guard against a scene that went flat: a picture of the effects differs from the plain
    // one, and the last (engine A: no effect, semi-transparent object over BG0) from the one before.
    let (shown, plain) = (shown_pictures(), expected()[0].clone());
    assert_eq!(
        (
            shown[1] != plain,
            shown[2] != shown[1],
            shown[9] != shown[8]
        ),
        (true, true, true)
    );
}

#[test]
fn given_blend_rom_when_run_for_14_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that these pictures are the right ones: the hash pins
    // the last of them.
    assert_frame_hash("blend", FRAMES);
}

#[test]
fn given_blend_rom_when_run_twice_for_14_frames_then_every_frame_hash_matches() {
    assert_deterministic("blend", FRAMES);
}

#[test]
fn given_blend_rom_when_saved_after_7_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    // Frame 7 is in the middle of the steps, so the effect registers are in the savestate.
    assert_savestate_round_trip("blend", 7, 7);
}
