// SPDX-License-Identifier: Apache-2.0
//! Vertex lighting, observed through our own `gxlight` test ROM (`testroms/gxlight/`). The ARM9 sets
//! a white light, a material and two triangles, each after its own NORMAL. The vertex colours end
//! in Vertex RAM, which no register reads, so this test reads them through the inspector. Expected
//! colours are worked out by hand from the model in `docs/hw/geometry-engine.md` ("Lighting"),
//! never from running the ROM. The light is (0, 0, -511), the material has diffuse white (31),
//! ambient red 4 and emission blue 2, and each channel is `emission + diffuse term + ambient term`:
//!
//! - Facing normal (0, 0, 511): `-(N . L) = 261121`, which shifts right by 9 to 510 of 512; the
//!   diffuse term is `31 x 31 x 510 / (31 x 512) = 30` in every channel. Red adds the ambient
//!   `4 x 31 / 31 = 4` (34, clamped to 31), green stays 30, blue adds the emission 2 (32, clamped to
//!   31): `(31, 30, 31)`, which is `31 | 30 << 5 | 31 << 10 = 7FDFh`.
//! - Normal turned away (0, 0, -511): the diffuse term is 0, so red is the ambient 4, green 0 and
//!   blue the emission 2: `4 | 2 << 10 = 0804h`.

use ninge_core::Machine;
use ninge_runner::golden::{assert_deterministic, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const FRAMES: u64 = 2;

fn boot_for(frames: u64) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path("gxlight")).expect("gxlight ROM is built"))
        .expect("gxlight ROM has a valid header");
    (0..frames).for_each(|_| machine.run_frame(Default::default()));
    machine
}

fn colors(machine: &Machine) -> Vec<u16> {
    machine
        .inspect()
        .gpu3d()
        .geometry_ram()
        .vertices
        .iter()
        .map(|vertex| vertex.color)
        .collect()
}

#[test]
fn given_the_gxlight_rom_when_it_runs_then_the_two_triangles_have_the_note_colours() {
    assert_eq!(
        colors(&boot_for(FRAMES)),
        [
            // TODO(hw-unverified): the fixed-point widths of the lighting equation are the note's model
            // (`geometry-engine.md`, "Unverified").
            0x7FDF, 0x7FDF, 0x7FDF, // facing the light: (31, 30, 31)
            0x0804, 0x0804, 0x0804, // turned away: ambient red 4, emission blue 2
        ]
    );
}

#[test]
fn given_the_gxlight_rom_when_it_runs_then_it_reports_done_with_its_sentinel() {
    let machine = boot_for(FRAMES);
    let inspect = machine.inspect();
    assert_eq!(
        (inspect.arm9_peek32(REC), inspect.arm9_peek32(REC + 0x7C)),
        (1, 0x494C_5847)
    );
}

#[test]
fn given_the_gxlight_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxlight", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxlight", 1, 1);
}
